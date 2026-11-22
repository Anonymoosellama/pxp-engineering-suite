// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-kernel-occt: Open CASCADE behind `pxp-kernel`, in a worker process
//!
//! [`OcctKernel`] implements `pxp_kernel::GeometryKernel` by sending each
//! call to a separate **worker process** that runs OCCT, over a versioned
//! request/response protocol (one JSON object per line on the worker's
//! stdin and stdout). OCCT is C++ and can abort its process; out of process,
//! an abort costs one call, never the document.
//!
//! ## Responsibility
//!
//! - Start the worker, check its protocol version, and restart it after it
//!   exits, hangs past the timeout, or answers out of order.
//! - Translate PXP's specifications into protocol requests and the replies
//!   into `OpResult`, `TopoHistory`, `MassProperties` and `KernelError`.
//! - Keep handles honest across restarts: every [`ShapeHandle`] carries the
//!   worker generation that made it, so a handle from a worker that died is
//!   `UnknownShape`, never a different shape that happens to have its number.
//!
//! ## Data flow
//!
//! ```text
//! KernelHost::op ─► OcctKernel::block/…/boolean ─► JSON line ─► worker (OCCT)
//!                          ▲                                       │
//!                          └──── OpResult { shape, history } ◄─ JSON line
//! ```
//!
//! ## The worker
//!
//! Protocol 1 is documented in `worker/pxp_occt_worker.py`, the bake-off
//! worker: OCCT 7.8 through its Python bindings (OCP, `pip install
//! cadquery-ocp`). It measures OCCT on the same corpus as SolidCraft
//! (ADR-0004). A shipped OCCT backend would speak the same protocol from a
//! C++ worker; this crate would not change.
//!
//! [`OcctConfig::from_env`] reads `PXP_OCCT_PYTHON` (the interpreter, default
//! `python3`), `PXP_OCCT_WORKER` (the worker script, default the copy in this
//! crate's source tree) and `PXP_OCCT_TIMEOUT_S` (seconds per call, default
//! 900).
//!
//! ## Invariants
//!
//! - One request in flight per worker; replies are matched by id, and a
//!   mismatch restarts the worker.
//! - Every topological operation returns the worker's complete history;
//!   `KernelHost::op` checks it as for any backend.
//!
//! ## Must never
//!
//! - Link OCCT into the main process.
//! - Be called directly by application crates (only shell crates wire
//!   backends; `tools/check_repo.py` enforces it).
#![forbid(unsafe_code)]

use pxp_kernel::{
    BackendId, BlockSpec, BodyKind, BooleanOp, Capabilities, ConeSpec, CylinderSpec,
    GeometryKernel, HistoryRecord, HistorySource, ImportedShape, KernelError, MassProperties,
    MergedCounts, OpResult, Relation, ShapeHandle, SphereSpec, StepImport, TopoHistory, TopoId,
    TopoKind,
};
use pxp_math::{Aabb, Point3};
use pxp_units::{Area, Length, Volume};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

/// Stage of the master prompt in which this crate's first real functionality
/// is scheduled. Shown by `pxp-cli status`.
pub const PLANNED_STAGE: u8 = 0;

/// This backend's identity.
pub const BACKEND: BackendId = BackendId("occt");

/// The protocol version this client speaks.
pub const PROTOCOL: u64 = 1;

/// Bits of a handle id that hold the worker's own shape number; the bits
/// above hold the worker generation.
const LOCAL_BITS: u32 = 40;

/// Operation names reported in [`Capabilities`].
const OPS: [&str; 16] = [
    "block",
    "cylinder",
    "cone",
    "sphere",
    "boolean",
    "topology",
    "body_kind",
    "mass_properties",
    "geometry_hash",
    "merged_counts",
    "export_native",
    "import_native",
    "bounding_box",
    "import_step",
    "export_step",
    "check_validity",
];

/// Every role a worker may name, as the static strings `pxp-kernel` uses.
fn role(name: &str) -> Option<&'static str> {
    BlockSpec::FACE_ROLES
        .iter()
        .chain(CylinderSpec::FACE_ROLES.iter())
        .chain(ConeSpec::FACE_ROLES.iter())
        .chain(SphereSpec::FACE_ROLES.iter())
        .copied()
        .find(|r| *r == name)
}

/// How to start the worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcctConfig {
    /// Interpreter that runs the worker script.
    pub python: PathBuf,
    /// The worker script.
    pub worker: PathBuf,
    /// Longest a single call may take before the worker is killed.
    pub timeout: Duration,
}

impl OcctConfig {
    /// From `PXP_OCCT_PYTHON`, `PXP_OCCT_WORKER` and `PXP_OCCT_TIMEOUT_S`
    /// (see the crate documentation for defaults).
    #[must_use]
    pub fn from_env() -> Self {
        let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty());
        Self {
            python: var("PXP_OCCT_PYTHON").map_or_else(|| PathBuf::from("python3"), PathBuf::from),
            worker: var("PXP_OCCT_WORKER").map_or_else(
                || {
                    PathBuf::from(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/worker/pxp_occt_worker.py"
                    ))
                },
                PathBuf::from,
            ),
            timeout: Duration::from_secs(
                var("PXP_OCCT_TIMEOUT_S")
                    .and_then(|v| v.to_str().and_then(|s| s.parse().ok()))
                    .unwrap_or(900),
            ),
        }
    }
}

/// A running worker.
struct Worker {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    next_id: u64,
}

impl Worker {
    fn kill(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct State {
    worker: Option<Worker>,
    /// Incremented at every start; part of every handle.
    generation: u64,
    /// The hello line of the current worker (diagnostics).
    hello: String,
}

/// OCCT in a worker process behind the `GeometryKernel` trait.
pub struct OcctKernel {
    config: OcctConfig,
    state: Mutex<State>,
}

impl std::fmt::Debug for OcctKernel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OcctKernel")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl Drop for OcctKernel {
    fn drop(&mut self) {
        if let Some(w) = self.lock().worker.take() {
            w.kill();
        }
    }
}

fn failure(op: &'static str, reason: impl Into<String>) -> KernelError {
    KernelError::GeometryFailure {
        op,
        reason: reason.into(),
        entities: Vec::new(),
    }
}

fn num(v: &Value, key: &str, op: &'static str) -> Result<f64, KernelError> {
    v.get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| failure(op, format!("worker reply lacks number '{key}'")))
}

fn count(v: &Value, key: &str, op: &'static str) -> Result<usize, KernelError> {
    v.get(key)
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| failure(op, format!("worker reply lacks count '{key}'")))
}

fn point(reply: &Value, key: &str, op: &'static str) -> Result<Point3, KernelError> {
    let items = reply
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| failure(op, format!("worker reply lacks point '{key}'")))?;
    let coords: Vec<f64> = items.iter().filter_map(Value::as_f64).collect();
    match coords.as_slice() {
        [px, py, pz] => Ok(Point3::new(*px, *py, *pz)),
        _ => Err(failure(op, format!("'{key}' is not a 3D point"))),
    }
}

fn kind_of(code: &str) -> Option<TopoKind> {
    match code {
        "v" => Some(TopoKind::Vertex),
        "e" => Some(TopoKind::Edge),
        "f" => Some(TopoKind::Face),
        _ => None,
    }
}

fn topo_id(code: &Value, index: &Value) -> Option<TopoId> {
    Some(TopoId {
        kind: kind_of(code.as_str()?)?,
        index: u32::try_from(index.as_u64()?).ok()?,
    })
}

fn source(v: &Value) -> Option<HistorySource> {
    match v.as_array()?.as_slice() {
        [tag, name] if tag == "r" => Some(HistorySource::Role(role(name.as_str()?)?)),
        [tag, operand, code, index] if tag == "i" => Some(HistorySource::Input {
            operand: u8::try_from(operand.as_u64()?).ok()?,
            entity: topo_id(code, index)?,
        }),
        _ => None,
    }
}

/// Converts the worker's history (see the worker's documentation) into a
/// `TopoHistory`. A record it cannot read is an error, not a gap.
fn history(v: &Value, op: &'static str) -> Result<TopoHistory, KernelError> {
    let bad = || failure(op, "worker history is malformed");
    let mut out = TopoHistory::default();
    for r in v.get("records").and_then(Value::as_array).ok_or_else(bad)? {
        let [code, index, rel, sources] = r.as_array().ok_or_else(bad)?.as_slice() else {
            return Err(bad());
        };
        let relation = match rel.as_str() {
            Some("m") => Relation::Modified,
            Some("g") => Relation::Generated,
            _ => return Err(bad()),
        };
        let sources = sources
            .as_array()
            .ok_or_else(bad)?
            .iter()
            .map(|s| source(s).ok_or_else(bad))
            .collect::<Result<Vec<_>, _>>()?;
        out.records.push(HistoryRecord {
            output: topo_id(code, index).ok_or_else(bad)?,
            relation,
            sources,
        });
    }
    for d in v.get("deleted").and_then(Value::as_array).ok_or_else(bad)? {
        let [operand, code, index] = d.as_array().ok_or_else(bad)?.as_slice() else {
            return Err(bad());
        };
        let operand = operand
            .as_u64()
            .and_then(|o| u8::try_from(o).ok())
            .ok_or_else(bad)?;
        out.deleted
            .push((operand, topo_id(code, index).ok_or_else(bad)?));
    }
    Ok(out)
}

impl OcctKernel {
    /// A kernel that starts its worker on first use.
    #[must_use]
    pub fn new(config: OcctConfig) -> Self {
        Self {
            config,
            state: Mutex::new(State {
                worker: None,
                generation: 0,
                hello: String::new(),
            }),
        }
    }

    /// Starts the worker now (rather than on first use) and returns its
    /// hello line, so a caller can tell at once whether OCCT is available.
    ///
    /// # Errors
    /// [`KernelError::Unsupported`] if the worker cannot be started or
    /// speaks another protocol.
    pub fn connect(&self) -> Result<String, KernelError> {
        let mut st = self.lock();
        self.ensure(&mut st)?;
        Ok(st.hello.clone())
    }

    /// Makes the worker exit abruptly, as an OCCT abort would. For tests of
    /// crash recovery only; the next call reports the crash and the one
    /// after it starts a new worker.
    #[doc(hidden)]
    pub fn abort_worker_for_test(&self) -> KernelError {
        match self.call("test_abort", |_, _| Ok(json!({}))) {
            Ok(_) => failure("test_abort", "the worker did not exit"),
            Err(e) => e,
        }
    }

    /// How many times a worker has been started.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.lock().generation
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn unavailable(&self, why: impl std::fmt::Display) -> KernelError {
        KernelError::Unsupported {
            op: "occt",
            reason: format!(
                "the OCCT worker ({} {}) is not available: {why}",
                self.config.python.display(),
                self.config.worker.display()
            ),
        }
    }

    fn ensure(&self, st: &mut State) -> Result<(), KernelError> {
        if st.worker.is_some() {
            return Ok(());
        }
        let mut child = Command::new(&self.config.python)
            .arg(&self.config.worker)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| self.unavailable(e))?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            return Err(self.unavailable("no pipes"));
        };
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let worker = Worker {
            child,
            stdin,
            lines: rx,
            next_id: 1,
        };
        let hello = match worker.lines.recv_timeout(Duration::from_mins(2)) {
            Ok(line) => line,
            Err(e) => {
                worker.kill();
                return Err(self.unavailable(format!("no hello ({e})")));
            }
        };
        let parsed: Value = serde_json::from_str(&hello).unwrap_or(Value::Null);
        let ok = parsed.get("v").and_then(Value::as_u64) == Some(PROTOCOL)
            && parsed.pointer("/hello/backend").and_then(Value::as_str) == Some("occt");
        if !ok {
            worker.kill();
            return Err(self.unavailable(format!("unexpected hello: {hello}")));
        }
        st.generation += 1;
        st.hello = hello;
        st.worker = Some(worker);
        Ok(())
    }

    fn handle(generation: u64, local: u64) -> ShapeHandle {
        ShapeHandle {
            backend: BACKEND,
            id: (generation << LOCAL_BITS) | local,
        }
    }

    /// The worker's own number for `shape`, if it belongs to the current
    /// worker.
    fn local(st: &State, shape: ShapeHandle) -> Result<u64, KernelError> {
        if shape.backend != BACKEND || shape.id >> LOCAL_BITS != st.generation {
            return Err(KernelError::UnknownShape(shape));
        }
        Ok(shape.id & ((1 << LOCAL_BITS) - 1))
    }

    /// Sends one request and waits for its reply.
    fn call(
        &self,
        op: &'static str,
        args: impl FnOnce(&Self, &State) -> Result<Value, KernelError>,
    ) -> Result<(Value, u64), KernelError> {
        let mut st = self.lock();
        self.ensure(&mut st)?;
        let args = args(self, &st)?;
        let generation = st.generation;
        let Some(w) = st.worker.as_mut() else {
            return Err(self.unavailable("not running"));
        };
        let id = w.next_id;
        w.next_id += 1;
        let line = json!({"v": PROTOCOL, "id": id, "op": op, "args": args}).to_string();
        let sent = writeln!(w.stdin, "{line}").and_then(|()| w.stdin.flush());
        let reply = match sent {
            Ok(()) => w.lines.recv_timeout(self.config.timeout),
            Err(_) => Err(RecvTimeoutError::Disconnected),
        };
        let reply = match reply {
            Ok(r) => r,
            Err(e) => {
                let timed_out = matches!(e, RecvTimeoutError::Timeout);
                if let Some(w) = st.worker.take() {
                    w.kill();
                }
                return Err(if timed_out {
                    failure(
                        op,
                        format!(
                            "the OCCT worker took longer than {} s and was restarted",
                            self.config.timeout.as_secs()
                        ),
                    )
                } else {
                    KernelError::BackendPanicked(format!(
                        "the OCCT worker exited during {op}; a new one starts on the next call"
                    ))
                });
            }
        };
        let v: Value = serde_json::from_str(&reply).unwrap_or(Value::Null);
        if v.get("id").and_then(Value::as_u64) != Some(id) {
            if let Some(w) = st.worker.take() {
                w.kill();
            }
            return Err(failure(
                op,
                format!("worker answered out of order: {reply}"),
            ));
        }
        if let Some(ok) = v.get("ok") {
            return Ok((ok.clone(), generation));
        }
        let err = v.get("err").cloned().unwrap_or(Value::Null);
        let message = err
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown worker error")
            .to_owned();
        Err(match err.get("kind").and_then(Value::as_str) {
            Some("invalid") => KernelError::InvalidInput(message),
            Some("empty") => KernelError::EmptyResult { op },
            Some("unsupported") => KernelError::Unsupported {
                op,
                reason: message,
            },
            Some("unknown_shape") => failure(op, format!("worker lost a shape: {message}")),
            _ => failure(op, message),
        })
    }

    fn shape_op(
        &self,
        op: &'static str,
        shape: ShapeHandle,
        extra: Value,
    ) -> Result<Value, KernelError> {
        self.call(op, |_, st| {
            let mut args = extra;
            if let Some(obj) = args.as_object_mut() {
                obj.insert("shape".into(), json!(Self::local(st, shape)?));
            }
            Ok(args)
        })
        .map(|(v, _)| v)
    }

    fn op_result(&self, op: &'static str, args: Value) -> Result<OpResult, KernelError> {
        let (v, generation) = self.call(op, |_, _| Ok(args))?;
        let local = v
            .get("shape")
            .and_then(Value::as_u64)
            .ok_or_else(|| failure(op, "worker reply lacks a shape"))?;
        let history = history(
            v.get("history")
                .ok_or_else(|| failure(op, "worker reply lacks a history"))?,
            op,
        )?;
        Ok(OpResult {
            shape: Self::handle(generation, local),
            history,
            warnings: Vec::new(),
        })
    }
}

fn xyz(p: Point3) -> Value {
    json!([p.x, p.y, p.z])
}

impl GeometryKernel for OcctKernel {
    fn backend_id(&self) -> BackendId {
        BACKEND
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            ops: OPS.into_iter().collect(),
        }
    }

    fn block(&self, spec: &BlockSpec) -> Result<OpResult, KernelError> {
        spec.validate()?;
        let t = spec.csys.to_absolute();
        let (x, y) = (t.rotation.cx, t.rotation.cy);
        self.op_result(
            "block",
            json!({
                "csys": {"origin": [t.translation.x, t.translation.y, t.translation.z],
                         "x": [x.x, x.y, x.z], "y": [y.x, y.y, y.z]},
                "size": [spec.x.mm(), spec.y.mm(), spec.z.mm()],
            }),
        )
    }

    fn cylinder(&self, spec: &CylinderSpec) -> Result<OpResult, KernelError> {
        spec.validate()?;
        let a = spec.axis.get();
        self.op_result(
            "cylinder",
            json!({"base": xyz(spec.base), "axis": [a.x, a.y, a.z],
                   "diameter": spec.diameter.mm(), "height": spec.height.mm()}),
        )
    }

    fn cone(&self, spec: &ConeSpec) -> Result<OpResult, KernelError> {
        spec.validate()?;
        let a = spec.axis.get();
        self.op_result(
            "cone",
            json!({"base": xyz(spec.base), "axis": [a.x, a.y, a.z],
                   "base_diameter": spec.base_diameter.mm(),
                   "top_diameter": spec.top_diameter.mm(), "height": spec.height.mm()}),
        )
    }

    fn sphere(&self, spec: &SphereSpec) -> Result<OpResult, KernelError> {
        spec.validate()?;
        self.op_result(
            "sphere",
            json!({"center": xyz(spec.center), "diameter": spec.diameter.mm()}),
        )
    }

    fn boolean(
        &self,
        op: BooleanOp,
        target: ShapeHandle,
        tool: ShapeHandle,
    ) -> Result<OpResult, KernelError> {
        let name = match op {
            BooleanOp::Unite => "unite",
            BooleanOp::Subtract => "subtract",
            BooleanOp::Intersect => "intersect",
        };
        let (v, generation) = self.call("boolean", |_, st| {
            Ok(json!({"op": name, "target": Self::local(st, target)?, "tool": Self::local(st, tool)?}))
        })?;
        let local = v
            .get("shape")
            .and_then(Value::as_u64)
            .ok_or_else(|| failure("boolean", "worker reply lacks a shape"))?;
        Ok(OpResult {
            shape: Self::handle(generation, local),
            history: history(
                v.get("history")
                    .ok_or_else(|| failure("boolean", "worker reply lacks a history"))?,
                "boolean",
            )?,
            warnings: Vec::new(),
        })
    }

    fn topology(&self, shape: ShapeHandle) -> Result<Vec<TopoId>, KernelError> {
        let v = self.shape_op("topology", shape, json!({}))?;
        let mut out = Vec::new();
        for (kind, key) in [
            (TopoKind::Vertex, "v"),
            (TopoKind::Edge, "e"),
            (TopoKind::Face, "f"),
            (TopoKind::Shell, "s"),
        ] {
            for i in 0..count(&v, key, "topology")? {
                let index =
                    u32::try_from(i).map_err(|_| failure("topology", "too many entities"))?;
                out.push(TopoId { kind, index });
            }
        }
        out.push(TopoId {
            kind: TopoKind::Body,
            index: 0,
        });
        Ok(out)
    }

    fn body_kind(&self, shape: ShapeHandle) -> Result<BodyKind, KernelError> {
        let v = self.shape_op("body_kind", shape, json!({}))?;
        match v.get("kind").and_then(Value::as_str) {
            Some("solid") => Ok(BodyKind::Solid),
            Some("sheet") => Ok(BodyKind::Sheet),
            Some("wire") => Ok(BodyKind::Wire),
            other => Err(failure("body_kind", format!("unknown body kind {other:?}"))),
        }
    }

    fn mass_properties(&self, shape: ShapeHandle) -> Result<MassProperties, KernelError> {
        let v = self.shape_op("mass_properties", shape, json!({}))?;
        Ok(MassProperties {
            volume: Volume::from_mm3(num(&v, "volume", "mass_properties")?),
            area: Area::from_mm2(num(&v, "area", "mass_properties")?),
            centroid: point(&v, "centroid", "mass_properties")?,
        })
    }

    fn geometry_hash(&self, shape: ShapeHandle, resolution: Length) -> Result<u64, KernelError> {
        let v = self.shape_op(
            "geometry_hash",
            shape,
            json!({"resolution": resolution.mm()}),
        )?;
        v.get("hash")
            .and_then(Value::as_str)
            .and_then(|h| u64::from_str_radix(h, 16).ok())
            .ok_or_else(|| failure("geometry_hash", "worker reply lacks a hash"))
    }

    fn merged_counts(&self, shape: ShapeHandle) -> Result<MergedCounts, KernelError> {
        let v = self.shape_op("merged_counts", shape, json!({}))?;
        Ok(MergedCounts {
            faces: count(&v, "faces", "merged_counts")?,
            edges: count(&v, "edges", "merged_counts")?,
            vertices: count(&v, "vertices", "merged_counts")?,
        })
    }

    /// OCCT's `BRep` text format (`BRepTools::Write`).
    fn export_native(&self, shape: ShapeHandle) -> Result<Vec<u8>, KernelError> {
        let v = self.shape_op("export_native", shape, json!({}))?;
        v.get("brep")
            .and_then(Value::as_str)
            .map(|s| s.as_bytes().to_vec())
            .ok_or_else(|| failure("export_native", "worker reply lacks BRep text"))
    }

    fn import_native(&self, bytes: &[u8]) -> Result<ShapeHandle, KernelError> {
        let text = std::str::from_utf8(bytes)
            .map_err(|_| KernelError::InvalidInput("not OCCT BRep text".into()))?
            .to_owned();
        let (v, generation) = self.call("import_native", move |_, _| Ok(json!({"brep": text})))?;
        v.get("shape")
            .and_then(Value::as_u64)
            .map(|local| Self::handle(generation, local))
            .ok_or_else(|| failure("import_native", "worker reply lacks a shape"))
    }

    fn bounding_box(&self, shape: ShapeHandle) -> Result<Aabb, KernelError> {
        let v = self.shape_op("bounding_box", shape, json!({}))?;
        Ok(Aabb::from_points([
            point(&v, "min", "bounding_box")?,
            point(&v, "max", "bounding_box")?,
        ]))
    }

    fn check_validity(&self, shape: ShapeHandle) -> Result<Vec<String>, KernelError> {
        let v = self.shape_op("check_validity", shape, json!({}))?;
        Ok(v.get("failed")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default())
    }

    /// OCCT's `STEPControl_Reader`, with its default shape healing.
    fn import_step(&self, text: &str) -> Result<StepImport, KernelError> {
        let text = text.to_owned();
        let (v, generation) = self.call("import_step", move |_, _| Ok(json!({"text": text})))?;
        let bad = || failure("import_step", "worker reply is malformed");
        let shapes = v
            .get("shapes")
            .and_then(Value::as_array)
            .ok_or_else(bad)?
            .iter()
            .map(|s| {
                Ok(ImportedShape {
                    shape: Self::handle(
                        generation,
                        s.get("shape").and_then(Value::as_u64).ok_or_else(bad)?,
                    ),
                    name: s
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned(),
                    closed: s.get("closed").and_then(Value::as_bool).unwrap_or(false),
                    mesh: s.get("mesh").and_then(Value::as_bool).unwrap_or(false),
                })
            })
            .collect::<Result<Vec<_>, KernelError>>()?;
        Ok(StepImport {
            shapes,
            parts: count(&v, "parts", "import_step")?,
            closed_parts: count(&v, "closed_parts", "import_step")?,
            schema: v
                .get("schema")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            warnings: v
                .get("warnings")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
        })
    }

    fn export_step(&self, shapes: &[ShapeHandle]) -> Result<String, KernelError> {
        let shapes = shapes.to_vec();
        let (v, _) = self.call("export_step", move |_, st| {
            let ids = shapes
                .iter()
                .map(|s| Self::local(st, *s))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(json!({"shapes": ids}))
        })?;
        v.get("text")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| failure("export_step", "worker reply lacks STEP text"))
    }

    fn release(&self, shape: ShapeHandle) {
        // A handle from an earlier worker died with it: nothing to free.
        let _ = self.shape_op("release", shape, json!({}));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_carry_the_generation() {
        let h = OcctKernel::handle(3, 17);
        let st = State {
            worker: None,
            generation: 3,
            hello: String::new(),
        };
        assert_eq!(OcctKernel::local(&st, h).unwrap(), 17);
        let old = OcctKernel::handle(2, 17);
        assert!(matches!(
            OcctKernel::local(&st, old),
            Err(KernelError::UnknownShape(_))
        ));
    }

    #[test]
    fn history_reads_roles_and_inputs() {
        let v: Value = serde_json::from_str(
            r#"{"records":[["f",0,"g",[["r","block.z_max"]]],["e",3,"m",[["i",1,"e",7]]]],
                "deleted":[[0,"f",2]]}"#,
        )
        .unwrap();
        let h = history(&v, "test").unwrap();
        assert_eq!(h.records.len(), 2);
        assert_eq!(
            h.records[0].sources,
            vec![HistorySource::Role("block.z_max")]
        );
        assert_eq!(
            h.deleted,
            vec![(
                0,
                TopoId {
                    kind: TopoKind::Face,
                    index: 2
                }
            )]
        );
        let unknown: Value =
            serde_json::from_str(r#"{"records":[["f",0,"g",[["r","nonsense"]]]],"deleted":[]}"#)
                .unwrap();
        assert!(history(&unknown, "test").is_err());
    }

    #[test]
    fn a_missing_worker_is_unsupported_not_a_crash() {
        let k = OcctKernel::new(OcctConfig {
            python: PathBuf::from("/nonexistent/python"),
            worker: PathBuf::from("/nonexistent/worker.py"),
            timeout: Duration::from_secs(5),
        });
        assert!(matches!(k.connect(), Err(KernelError::Unsupported { .. })));
    }
}
