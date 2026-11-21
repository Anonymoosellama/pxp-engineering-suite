// SPDX-License-Identifier: LGPL-2.1-or-later
//! Maps SolidCraft's operation history onto `pxp_kernel::TopoHistory`.
//!
//! ## What SolidCraft reports
//!
//! Every result body carries a `History` (`solidcore_record::brep::history`):
//! one `FaceProv` per result face (its origins: input faces, profile
//! segments, sweep caps, input edges for blends, input faces for offsets;
//! and whether it was kept, modified, split, merged or generated) and one
//! `EdgeProv` per result edge (the faces it separates and its origins: input
//! edges, or the pair of input faces whose meeting made it). There are no
//! vertex records, and an edge new to the operation between faces new to
//! the operation (every edge of a primitive) has no origin.
//!
//! ## How the gaps are filled ("derived records")
//!
//! `pxp-kernel` requires a record for every vertex, edge and face
//! (`TopoHistory::check_complete`). The adapter fills SolidCraft's gaps from
//! exact adjacency in the result's B-rep record, never from geometry
//! guesses:
//!
//! - **Edge with no origin** → `Generated` from the sources of the faces it
//!   separates, in face order. A block's top-front edge is generated from
//!   `block.z_max` and `block.y_min`: the classic "edge between faces" name.
//! - **Vertex** → if the input edges that its incident edges came from all
//!   share exactly one endpoint at the vertex's position (within the
//!   vertices' tolerances), `Modified` from that input vertex; otherwise
//!   `Generated` from the sources of the faces around it.
//! - **Deleted** → input faces, edges and vertices that no result entity
//!   names as a source.
//!
//! Faces always use SolidCraft's own records. If SolidCraft ever returns a
//! body with no history, the adapter returns an empty history and
//! `KernelHost::op` rejects the result (`IncompleteHistory`): a gap becomes a
//! visible failure, never a silent workaround.
//!
//! Complexity: O(V + E + F + Σ loop lengths) per result, plus, for vertex
//! matching, O(incident edges × origins) per vertex.

use pxp_kernel::{
    HistoryRecord, HistorySource, KernelError, Relation, TopoHistory, TopoId, TopoKind,
};
use solidcore_record::brep::{Brep, Loop};
use solidcraft_kernel::Body;
use solidcraft_kernel::history::{Change, EdgeOrigin, FaceOrigin, History};
use std::collections::BTreeSet;

/// Role names for faces that have no input: sweep caps of operations other
/// than primitives (primitives name their faces with their own roles).
/// Part of the naming contract: never rename.
const CAP_START: &str = "sweep.cap_start";
const CAP_END: &str = "sweep.cap_end";

/// Relation and sources for one output entity.
type Entry = (Relation, Vec<HistorySource>);

fn failure(reason: impl Into<String>) -> KernelError {
    KernelError::GeometryFailure {
        op: "history",
        reason: reason.into(),
        entities: Vec::new(),
    }
}

/// A `TopoId`, checking that the index fits.
pub(crate) fn topo(kind: TopoKind, index: usize) -> Result<TopoId, KernelError> {
    let index = u32::try_from(index).map_err(|_| failure("more than 2^32 entities"))?;
    Ok(TopoId { kind, index })
}

fn operand_u8(operand: usize) -> Result<u8, KernelError> {
    u8::try_from(operand).map_err(|_| failure("more than 255 operands"))
}

fn face_origin(origin: FaceOrigin) -> Result<HistorySource, KernelError> {
    Ok(match origin {
        FaceOrigin::Face { operand, face } | FaceOrigin::Offset { operand, face } => {
            HistorySource::Input {
                operand: operand_u8(operand)?,
                entity: topo(TopoKind::Face, face)?,
            }
        }
        FaceOrigin::Edge { operand, edge } => HistorySource::Input {
            operand: operand_u8(operand)?,
            entity: topo(TopoKind::Edge, edge)?,
        },
        FaceOrigin::Profile { segment } => HistorySource::Profile {
            curve: u32::try_from(segment).map_err(|_| failure("profile index overflow"))?,
        },
        FaceOrigin::Cap { end } => HistorySource::Role(if end { CAP_END } else { CAP_START }),
    })
}

/// Pushes `items` onto `out`, skipping ones already present (keeps first
/// occurrence order, so the result is deterministic).
fn extend_unique(out: &mut Vec<HistorySource>, items: &[HistorySource]) {
    for s in items {
        if !out.contains(s) {
            out.push(s.clone());
        }
    }
}

/// Exact adjacency read from the B-rep record.
struct Adjacency {
    /// Faces each edge bounds (sorted, unique).
    edge_faces: Vec<BTreeSet<usize>>,
    /// Edges meeting at each vertex.
    vertex_edges: Vec<BTreeSet<usize>>,
    /// Faces around each vertex (via its edges, or a vertex loop).
    vertex_faces: Vec<BTreeSet<usize>>,
}

impl Adjacency {
    fn of(rec: &Brep) -> Self {
        let mut edge_faces = vec![BTreeSet::new(); rec.edges.len()];
        let mut vertex_edges = vec![BTreeSet::new(); rec.vertices.len()];
        let mut vertex_faces = vec![BTreeSet::new(); rec.vertices.len()];
        for (e, edge) in rec.edges.iter().enumerate() {
            let faces: BTreeSet<usize> = edge.uses.iter().map(|u| u.face as usize).collect();
            for &v in &edge.vertices {
                if let Some(set) = vertex_edges.get_mut(v as usize) {
                    set.insert(e);
                }
                if let Some(set) = vertex_faces.get_mut(v as usize) {
                    set.extend(faces.iter().copied());
                }
            }
            if let Some(slot) = edge_faces.get_mut(e) {
                *slot = faces;
            }
        }
        for (f, face) in rec.faces.iter().enumerate() {
            for l in &face.loops {
                if let Loop::Vertex { vertex } = l
                    && let Some(set) = vertex_faces.get_mut(*vertex as usize)
                {
                    set.insert(f);
                }
            }
        }
        Self {
            edge_faces,
            vertex_edges,
            vertex_faces,
        }
    }
}

/// Sources of the faces in `set`, in face order, without repeats.
fn sources_of_faces(faces: &[Entry], set: &BTreeSet<usize>) -> Vec<HistorySource> {
    let mut out = Vec::new();
    for &f in set {
        if let Some((_, s)) = faces.get(f) {
            extend_unique(&mut out, s);
        }
    }
    out
}

/// History of a primitive: every face gets its role, edges and vertices are
/// derived from adjacency (SolidCraft gives primitive edges no origin).
pub(crate) fn primitive(body: &Body, roles: &[&'static str]) -> Result<TopoHistory, KernelError> {
    let rec = body.record();
    if roles.len() != rec.faces.len() {
        return Err(failure(format!(
            "{} face roles for {} faces",
            roles.len(),
            rec.faces.len()
        )));
    }
    let faces: Vec<Entry> = roles
        .iter()
        .map(|&r| (Relation::Generated, vec![HistorySource::Role(r)]))
        .collect();
    let adj = Adjacency::of(rec);
    let edges: Vec<Entry> = adj
        .edge_faces
        .iter()
        .map(|set| (Relation::Generated, sources_of_faces(&faces, set)))
        .collect();
    let vertices: Vec<Entry> = adj
        .vertex_faces
        .iter()
        .map(|set| (Relation::Generated, sources_of_faces(&faces, set)))
        .collect();
    assemble(&faces, &edges, &vertices, Vec::new())
}

/// The face roles of a primitive, read from SolidCraft's history with
/// `role` translating each face's single origin. Fails if a face has no
/// origin or `role` does not know it.
pub(crate) fn roles_from_history(
    body: &Body,
    role: impl Fn(FaceOrigin) -> Option<&'static str>,
) -> Result<Vec<&'static str>, KernelError> {
    let h = body
        .history()
        .ok_or_else(|| failure("SolidCraft returned a primitive without history"))?;
    h.faces
        .iter()
        .enumerate()
        .map(|(i, f)| {
            f.origins
                .first()
                .copied()
                .and_then(&role)
                .ok_or_else(|| failure(format!("primitive face {i} has unknown origin {f:?}")))
        })
        .collect()
}

/// History of an operation on `inputs` (operand 0 = target, 1 = tool).
pub(crate) fn operation(body: &Body, inputs: &[&Body]) -> Result<TopoHistory, KernelError> {
    let rec = body.record();
    let Some(h) = body.history() else {
        // Let `KernelHost::op` report the gap as IncompleteHistory.
        return Ok(TopoHistory::default());
    };
    if h.faces.len() != rec.faces.len() || h.edges.len() != rec.edges.len() {
        return Err(failure("SolidCraft history does not match the result body"));
    }
    if h.operands != inputs.len() {
        return Err(failure(format!(
            "history names {} operands, the operation had {}",
            h.operands,
            inputs.len()
        )));
    }
    let faces: Vec<Entry> = h
        .faces
        .iter()
        .map(|fp| {
            let relation = if fp.change == Change::Generated {
                Relation::Generated
            } else {
                Relation::Modified
            };
            let sources = fp
                .origins
                .iter()
                .map(|&o| face_origin(o))
                .collect::<Result<Vec<_>, _>>()?;
            Ok((relation, sources))
        })
        .collect::<Result<_, KernelError>>()?;
    let adj = Adjacency::of(rec);
    let edges = operation_edges(h, &faces, &adj)?;
    let inputs_rec: Vec<&Brep> = inputs.iter().map(|b| b.record()).collect();
    let (vertices, matched) = operation_vertices(rec, h, &faces, &adj, &inputs_rec)?;
    let deleted = deleted(h, &inputs_rec, &matched)?;
    assemble(&faces, &edges, &vertices, deleted)
}

fn operation_edges(
    h: &History,
    faces: &[Entry],
    adj: &Adjacency,
) -> Result<Vec<Entry>, KernelError> {
    h.edges
        .iter()
        .zip(&adj.edge_faces)
        .map(|(ep, around)| {
            if ep.origins.is_empty() {
                return Ok((Relation::Generated, sources_of_faces(faces, around)));
            }
            let mut sources = Vec::new();
            let mut all_edges = true;
            for o in &ep.origins {
                match o {
                    EdgeOrigin::Edge { operand, edge } => extend_unique(
                        &mut sources,
                        &[HistorySource::Input {
                            operand: operand_u8(*operand)?,
                            entity: topo(TopoKind::Edge, *edge)?,
                        }],
                    ),
                    EdgeOrigin::Meet { a, b } => {
                        all_edges = false;
                        for &fo in a.origins().iter().chain(b.origins()) {
                            extend_unique(&mut sources, &[face_origin(fo)?]);
                        }
                    }
                }
            }
            let relation = if all_edges {
                Relation::Modified
            } else {
                Relation::Generated
            };
            Ok((relation, sources))
        })
        .collect()
}

/// Input vertices matched per output vertex: `(operand, input vertex)`.
type Matched = BTreeSet<(usize, usize)>;

fn operation_vertices(
    rec: &Brep,
    h: &History,
    faces: &[Entry],
    adj: &Adjacency,
    inputs: &[&Brep],
) -> Result<(Vec<Entry>, Matched), KernelError> {
    let mut matched = Matched::new();
    let mut out = Vec::with_capacity(rec.vertices.len());
    for (v, vertex) in rec.vertices.iter().enumerate() {
        let incident = adj.vertex_edges.get(v).cloned().unwrap_or_default();
        let mut sources = Vec::new();
        for (operand, input) in inputs.iter().enumerate() {
            let mut candidates: Option<BTreeSet<usize>> = None;
            for e in &incident {
                let Some(ep) = h.edges.get(*e) else { continue };
                for o in &ep.origins {
                    if let EdgeOrigin::Edge { operand: op, edge } = o
                        && *op == operand
                        && let Some(ie) = input.edges.get(*edge)
                    {
                        let ends: BTreeSet<usize> =
                            ie.vertices.iter().map(|&x| x as usize).collect();
                        candidates = Some(match candidates {
                            None => ends,
                            Some(c) => c.intersection(&ends).copied().collect(),
                        });
                    }
                }
            }
            let at_vertex: Vec<usize> = candidates
                .unwrap_or_default()
                .into_iter()
                .filter(|&iv| {
                    input.vertices.get(iv).is_some_and(|ivx| {
                        let d2: f64 = (0..3)
                            .map(|k| {
                                let a = vertex.point.get(k).copied().unwrap_or(f64::NAN);
                                let b = ivx.point.get(k).copied().unwrap_or(f64::NAN);
                                (a - b) * (a - b)
                            })
                            .sum();
                        let tol = (vertex.tolerance + ivx.tolerance).max(1e-9);
                        d2.sqrt() <= tol
                    })
                })
                .collect();
            if let [iv] = at_vertex.as_slice() {
                matched.insert((operand, *iv));
                sources.push(HistorySource::Input {
                    operand: operand_u8(operand)?,
                    entity: topo(TopoKind::Vertex, *iv)?,
                });
            }
        }
        if sources.is_empty() {
            let around = adj.vertex_faces.get(v).cloned().unwrap_or_default();
            out.push((Relation::Generated, sources_of_faces(faces, &around)));
        } else {
            out.push((Relation::Modified, sources));
        }
    }
    Ok((out, matched))
}

fn deleted(
    h: &History,
    inputs: &[&Brep],
    matched: &Matched,
) -> Result<Vec<(u8, TopoId)>, KernelError> {
    let mut used_faces = BTreeSet::new();
    let mut used_edges = BTreeSet::new();
    for fp in &h.faces {
        for o in &fp.origins {
            match *o {
                FaceOrigin::Face { operand, face } | FaceOrigin::Offset { operand, face } => {
                    used_faces.insert((operand, face));
                }
                FaceOrigin::Edge { operand, edge } => {
                    used_edges.insert((operand, edge));
                }
                FaceOrigin::Profile { .. } | FaceOrigin::Cap { .. } => {}
            }
        }
    }
    for ep in &h.edges {
        for o in &ep.origins {
            if let EdgeOrigin::Edge { operand, edge } = *o {
                used_edges.insert((operand, edge));
            }
        }
    }
    let mut out = Vec::new();
    for (operand, rec) in inputs.iter().enumerate() {
        let op = operand_u8(operand)?;
        for f in (0..rec.faces.len()).filter(|f| !used_faces.contains(&(operand, *f))) {
            out.push((op, topo(TopoKind::Face, f)?));
        }
        for e in (0..rec.edges.len()).filter(|e| !used_edges.contains(&(operand, *e))) {
            out.push((op, topo(TopoKind::Edge, e)?));
        }
        for v in (0..rec.vertices.len()).filter(|v| !matched.contains(&(operand, *v))) {
            out.push((op, topo(TopoKind::Vertex, v)?));
        }
    }
    Ok(out)
}

/// Builds the history: faces, then edges, then vertices, each in index
/// order (deterministic).
fn assemble(
    faces: &[Entry],
    edges: &[Entry],
    vertices: &[Entry],
    deleted: Vec<(u8, TopoId)>,
) -> Result<TopoHistory, KernelError> {
    let mut records = Vec::with_capacity(faces.len() + edges.len() + vertices.len());
    for (kind, entries) in [
        (TopoKind::Face, faces),
        (TopoKind::Edge, edges),
        (TopoKind::Vertex, vertices),
    ] {
        for (i, (relation, sources)) in entries.iter().enumerate() {
            records.push(HistoryRecord {
                output: topo(kind, i)?,
                relation: *relation,
                sources: sources.clone(),
            });
        }
    }
    Ok(TopoHistory { records, deleted })
}
