// SPDX-License-Identifier: LGPL-2.1-or-later
//! Stage 0 features: the datum CSYS and the four primitives (block,
//! cylinder, cone, sphere), created in a transaction and regenerated through
//! the kernel trait by [`crate::Modeling`]'s recompute.
//!
//! ## Object schema (persisted: names never change)
//!
//! | Kind | Properties | Dependencies |
//! | --- | --- | --- |
//! | `modeling.datum_csys` | `origin_x/y/z` (length), `x_axis_x/y/z`, `y_axis_x/y/z` (numbers; Y is a hint, made orthogonal) | none |
//! | `modeling.block` | `x_length`, `y_length`, `z_length`; optional `csys` (reference to a datum CSYS) | each referenced expression (`param`), the datum (`csys`) |
//! | `modeling.cylinder` | `base_x/y/z`, `axis_x/y/z`, `diameter`, `height` | referenced expressions |
//! | `modeling.cone` | `base_x/y/z`, `axis_x/y/z`, `base_diameter`, `top_diameter`, `height` | referenced expressions |
//! | `modeling.sphere` | `center_x/y/z`, `diameter` | referenced expressions |
//!
//! Every parameter is either a quantity or a reference to an expression
//! object, as in NX where each dialog value becomes an expression (`p0`,
//! `p1`, …). The `create_*` functions follow NX: each value becomes an
//! expression named with the next free `pN`, and the feature references it.
//!
//! ## Results (written by recompute)
//!
//! - `geometry_hash` (integer: the backend's `geometry_hash` at
//!   [`pxp_kernel::DEFAULT_HASH_RESOLUTION`], bits reinterpreted as `i64`),
//! - `kernel` (text: the backend that computed it),
//! - payload: the exact B-rep in the stored-shape envelope
//!   (`pxp_kernel::encode_stored_shape`), so a reopened part has its
//!   geometry before anything is recomputed.
//!
//! The kernel shape is released as soon as hash and B-rep are taken: the
//! document, not the kernel, owns the geometry.

use pxp_doc::expr::VALUE_PROP;
use pxp_doc::{
    DocError, NewObject, Object, ObjectId, ObjectKind, RecomputeOutput, Snapshot, Tx, Value,
};
use pxp_kernel::{
    BlockSpec, ConeSpec, CylinderSpec, DEFAULT_HASH_RESOLUTION, GeometryKernel, KernelError,
    KernelHost, OpResult, SphereSpec, encode_stored_shape,
};
use pxp_math::{Csys, Point3, UnitVec3, Vec3};
use pxp_units::{Length, Quantity};
use std::collections::BTreeMap;

/// Datum coordinate system.
pub const DATUM_CSYS: ObjectKind = ObjectKind::from_static("modeling.datum_csys");
/// Block primitive.
pub const BLOCK: ObjectKind = ObjectKind::from_static("modeling.block");
/// Cylinder primitive.
pub const CYLINDER: ObjectKind = ObjectKind::from_static("modeling.cylinder");
/// Cone primitive.
pub const CONE: ObjectKind = ObjectKind::from_static("modeling.cone");
/// Sphere primitive.
pub const SPHERE: ObjectKind = ObjectKind::from_static("modeling.sphere");

/// Result property: geometry hash.
pub const GEOMETRY_HASH: &str = "geometry_hash";
/// Result property: backend that computed the geometry.
pub const KERNEL: &str = "kernel";
/// Optional block property: the datum CSYS it sits on.
pub const CSYS: &str = "csys";
/// Dependency role for a referenced expression.
pub const PARAM_ROLE: &str = "param";
/// Dependency role for the block's datum CSYS.
pub const CSYS_ROLE: &str = "csys";

/// True for the kinds this module regenerates.
#[must_use]
pub fn is_primitive_kind(kind: &ObjectKind) -> bool {
    [DATUM_CSYS, BLOCK, CYLINDER, CONE, SPHERE].contains(kind)
}

/// True for the kinds that produce a solid body.
#[must_use]
pub fn is_solid_kind(kind: &ObjectKind) -> bool {
    [BLOCK, CYLINDER, CONE, SPHERE].contains(kind)
}

// ---- reading parameters -------------------------------------------------

fn quantity(snap: &Snapshot, obj: &Object, key: &str) -> Result<Quantity, String> {
    match obj.prop(key) {
        Some(Value::Quantity(q)) => Ok(*q),
        Some(Value::Ref(id)) => match snap.get(*id).and_then(|e| e.prop(VALUE_PROP)) {
            Some(Value::Quantity(q)) => Ok(*q),
            _ => Err(format!("{key}: expression {id} has no value")),
        },
        Some(_) => Err(format!("{key} is not a value or an expression")),
        None => Err(format!("missing parameter {key}")),
    }
}

fn length(snap: &Snapshot, obj: &Object, key: &str) -> Result<Length, String> {
    let q = quantity(snap, obj, key)?;
    Length::try_from(q).map_err(|_| format!("{key} must be a length (for example 10 mm), got {q}"))
}

fn number(snap: &Snapshot, obj: &Object, key: &str) -> Result<f64, String> {
    let q = quantity(snap, obj, key)?;
    if q.dim() == pxp_units::Dim::DIMENSIONLESS {
        Ok(q.value())
    } else {
        Err(format!("{key} must be a plain number, got {q}"))
    }
}

fn point(snap: &Snapshot, obj: &Object, prefix: &str) -> Result<Point3, String> {
    Ok(Point3::from_lengths(
        length(snap, obj, &format!("{prefix}_x"))?,
        length(snap, obj, &format!("{prefix}_y"))?,
        length(snap, obj, &format!("{prefix}_z"))?,
    ))
}

fn vector(snap: &Snapshot, obj: &Object, prefix: &str) -> Result<Vec3, String> {
    Ok(Vec3::new(
        number(snap, obj, &format!("{prefix}_x"))?,
        number(snap, obj, &format!("{prefix}_y"))?,
        number(snap, obj, &format!("{prefix}_z"))?,
    ))
}

fn direction(snap: &Snapshot, obj: &Object, prefix: &str) -> Result<UnitVec3, String> {
    vector(snap, obj, prefix)?
        .normalized()
        .ok_or_else(|| format!("{prefix} direction has zero length"))
}

fn datum_csys(snap: &Snapshot, obj: &Object) -> Result<Csys, String> {
    Csys::new(
        point(snap, obj, "origin")?,
        vector(snap, obj, "x_axis")?,
        vector(snap, obj, "y_axis")?,
    )
    .map_err(|e| format!("datum CSYS: {e}"))
}

fn block_spec(snap: &Snapshot, obj: &Object) -> Result<BlockSpec, String> {
    let csys = match obj.prop(CSYS) {
        None => Csys::ABSOLUTE,
        Some(Value::Ref(id)) => {
            let datum = snap
                .get(*id)
                .filter(|d| d.kind == DATUM_CSYS)
                .ok_or_else(|| format!("csys {id} is not a datum CSYS"))?;
            datum_csys(snap, datum)?
        }
        Some(_) => return Err("csys must reference a datum CSYS".into()),
    };
    Ok(BlockSpec {
        csys,
        x: length(snap, obj, "x_length")?,
        y: length(snap, obj, "y_length")?,
        z: length(snap, obj, "z_length")?,
    })
}

// ---- recompute ----------------------------------------------------------

/// Runs one solid-producing kernel operation and turns its result into the
/// feature's stored results (module docs, "Results").
fn solid_result(
    kernel: &KernelHost,
    op: impl FnOnce(&dyn GeometryKernel) -> Result<OpResult, KernelError>,
) -> Result<RecomputeOutput, String> {
    let r = kernel.op(op).map_err(|e| e.to_string())?;
    let shape = r.shape;
    let taken = kernel
        .call(|k| k.geometry_hash(shape, DEFAULT_HASH_RESOLUTION))
        .and_then(|hash| {
            let native = kernel.call(|k| k.export_native(shape))?;
            Ok((hash, encode_stored_shape(kernel.backend_id(), &native)?))
        });
    let _ = kernel.call(|k| {
        k.release(shape);
        Ok(())
    });
    let (hash, stored) = taken.map_err(|e| e.to_string())?;
    let mut props = BTreeMap::new();
    props.insert(GEOMETRY_HASH.to_owned(), Value::Int(hash.cast_signed()));
    props.insert(
        KERNEL.to_owned(),
        Value::Text(kernel.backend_id().0.to_owned()),
    );
    let warning = (!r.warnings.is_empty()).then(|| {
        r.warnings
            .iter()
            .map(|w| w.message.as_str())
            .collect::<Vec<_>>()
            .join("; ")
    });
    Ok(RecomputeOutput {
        props,
        payload: Some(stored),
        warning,
    })
}

/// Regenerates a primitive or datum. `kernel` is `None` when the shell has
/// connected no geometry backend: solids then refuse with that reason.
///
/// # Errors
/// A reason shown in the Part Navigator.
pub fn recompute(
    kernel: Option<&KernelHost>,
    snap: &Snapshot,
    obj: &Object,
) -> Result<RecomputeOutput, String> {
    if obj.kind == DATUM_CSYS {
        datum_csys(snap, obj)?;
        return Ok(RecomputeOutput::default());
    }
    let kernel = kernel.ok_or("no geometry kernel is connected")?;
    if obj.kind == BLOCK {
        let spec = block_spec(snap, obj)?;
        solid_result(kernel, |k| k.block(&spec))
    } else if obj.kind == CYLINDER {
        let spec = CylinderSpec {
            base: point(snap, obj, "base")?,
            axis: direction(snap, obj, "axis")?,
            diameter: length(snap, obj, "diameter")?,
            height: length(snap, obj, "height")?,
        };
        solid_result(kernel, |k| k.cylinder(&spec))
    } else if obj.kind == CONE {
        let spec = ConeSpec {
            base: point(snap, obj, "base")?,
            axis: direction(snap, obj, "axis")?,
            base_diameter: length(snap, obj, "base_diameter")?,
            top_diameter: length(snap, obj, "top_diameter")?,
            height: length(snap, obj, "height")?,
        };
        solid_result(kernel, |k| k.cone(&spec))
    } else if obj.kind == SPHERE {
        let spec = SphereSpec {
            center: point(snap, obj, "center")?,
            diameter: length(snap, obj, "diameter")?,
        };
        solid_result(kernel, |k| k.sphere(&spec))
    } else {
        Err(format!("'{}' is not a primitive", obj.kind))
    }
}

// ---- creation -----------------------------------------------------------

/// Builds a feature whose parameters become NX-style expressions.
struct Builder<'t, 'a> {
    tx: &'t mut Tx<'a>,
    spec: NewObject,
}

impl<'t, 'a> Builder<'t, 'a> {
    fn new(tx: &'t mut Tx<'a>, kind: ObjectKind, name: &str) -> Self {
        Self {
            tx,
            spec: NewObject::new(kind, crate::APP_ID).name(name),
        }
    }

    /// The next free expression name `pN`.
    fn next_name(&self) -> String {
        (0_u32..=u32::MAX)
            .map(|n| format!("p{n}"))
            .find(|n| self.tx.expression(n).is_none())
            .unwrap_or_else(|| "p".to_owned())
    }

    /// Stores `formula` as a new expression and references it as `key`.
    fn param(mut self, key: &str, formula: &str) -> Result<Self, DocError> {
        let name = self.next_name();
        let id = self.tx.set_expression(&name, formula)?;
        self.spec = self.spec.prop(key, Value::Ref(id)).dep(id, PARAM_ROLE);
        Ok(self)
    }

    fn params(mut self, prefix: &str, formulas: &[&str; 3]) -> Result<Self, DocError> {
        for (axis, f) in ["x", "y", "z"].iter().zip(formulas) {
            self = self.param(&format!("{prefix}_{axis}"), f)?;
        }
        Ok(self)
    }

    fn create(self) -> Result<ObjectId, DocError> {
        self.tx.create(self.spec)
    }
}

/// Creates a datum CSYS at `origin` (lengths) with axes `x_axis` and a Y
/// hint `y_axis` (plain numbers). Values are stored directly: datum CSYS
/// placement is not parametric in Stage 0.
///
/// # Errors
/// A document error from the transaction.
pub fn create_datum_csys(
    tx: &mut Tx<'_>,
    name: &str,
    origin: Point3,
    x_axis: Vec3,
    y_axis: Vec3,
) -> Result<ObjectId, DocError> {
    let len = |v: f64| Value::Quantity(Length::from_mm(v).into());
    let num = |v: f64| Value::Quantity(Quantity::number(v));
    let spec = NewObject::new(DATUM_CSYS, crate::APP_ID)
        .name(name)
        .prop("origin_x", len(origin.x))
        .prop("origin_y", len(origin.y))
        .prop("origin_z", len(origin.z))
        .prop("x_axis_x", num(x_axis.x))
        .prop("x_axis_y", num(x_axis.y))
        .prop("x_axis_z", num(x_axis.z))
        .prop("y_axis_x", num(y_axis.x))
        .prop("y_axis_y", num(y_axis.y))
        .prop("y_axis_z", num(y_axis.z));
    tx.create(spec)
}

/// Creates a block from three length formulas (`"10 mm"`, `"width * 2"`),
/// optionally on a datum CSYS.
///
/// # Errors
/// A document error (bad formula, unknown name, unit error).
pub fn create_block(
    tx: &mut Tx<'_>,
    name: &str,
    lengths: [&str; 3],
    csys: Option<ObjectId>,
) -> Result<ObjectId, DocError> {
    let [x, y, z] = lengths;
    let mut b = Builder::new(tx, BLOCK, name)
        .param("x_length", x)?
        .param("y_length", y)?
        .param("z_length", z)?;
    if let Some(c) = csys {
        b.spec = b.spec.prop(CSYS, Value::Ref(c)).dep(c, CSYS_ROLE);
    }
    b.create()
}

/// Creates a cylinder: base point and axis formulas, diameter and height.
///
/// # Errors
/// A document error.
pub fn create_cylinder(
    tx: &mut Tx<'_>,
    name: &str,
    base: [&str; 3],
    axis: [&str; 3],
    diameter: &str,
    height: &str,
) -> Result<ObjectId, DocError> {
    Builder::new(tx, CYLINDER, name)
        .params("base", &base)?
        .params("axis", &axis)?
        .param("diameter", diameter)?
        .param("height", height)?
        .create()
}

/// Creates a cone or truncated cone.
///
/// # Errors
/// A document error.
pub fn create_cone(
    tx: &mut Tx<'_>,
    name: &str,
    base: [&str; 3],
    axis: [&str; 3],
    diameters: [&str; 2],
    height: &str,
) -> Result<ObjectId, DocError> {
    let [base_d, top_d] = diameters;
    Builder::new(tx, CONE, name)
        .params("base", &base)?
        .params("axis", &axis)?
        .param("base_diameter", base_d)?
        .param("top_diameter", top_d)?
        .param("height", height)?
        .create()
}

/// Creates a sphere.
///
/// # Errors
/// A document error.
pub fn create_sphere(
    tx: &mut Tx<'_>,
    name: &str,
    center: [&str; 3],
    diameter: &str,
) -> Result<ObjectId, DocError> {
    Builder::new(tx, SPHERE, name)
        .params("center", &center)?
        .param("diameter", diameter)?
        .create()
}
