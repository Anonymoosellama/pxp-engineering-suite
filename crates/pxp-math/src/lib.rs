// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-math: geometry maths, coordinate systems, tolerances, stable hashing
//!
//! ## Responsibility
//!
//! - 3D vectors and points ([`Vec3`], [`Point3`], [`UnitVec3`]), rigid
//!   transforms ([`Transform`], [`Mat3`]) and coordinate systems ([`Csys`]:
//!   the WCS and datum CSYS of Stage 0 and 1A are built on it).
//! - Axis-aligned bounding boxes ([`Aabb`]).
//! - Per-part modeling tolerances ([`Tolerances`]), stored in each `.pxp`
//!   and shown in part preferences.
//! - A stable, platform-independent hasher ([`StableHasher`]) for geometry
//!   hashes that are persisted in files and compared across operating
//!   systems.
//!
//! ## Units
//!
//! Coordinates are plain `f64` **millimetres** (the internal length unit).
//! Typed [`pxp_units::Length`] values are used at API boundaries
//! ([`Point3::from_lengths`], [`Tolerances`]); inside numeric kernels a raw
//! `f64` in mm keeps the maths readable. Angles are radians, typed as
//! [`pxp_units::Angle`] wherever they cross an API.
//!
//! ## Data flow
//!
//! Foundation crate. Depends only on `pxp-units`; used by every geometry,
//! CAM and CAE crate.
//!
//! ## Invariants
//!
//! - A [`UnitVec3`] always has length 1 within 1e-12; it can only be built by
//!   normalising a non-degenerate vector.
//! - A [`Csys`] is always right-handed and orthonormal.
//! - [`StableHasher`] output depends only on the bytes written, never on the
//!   platform, Rust version or process (unlike `std`'s `DefaultHasher`).
//!
//! ## Determinism note
//!
//! IEEE-754 `+ - * /` and `sqrt` are bit-exact on every target we ship, so
//! code built from them is deterministic across OSes. Transcendental
//! functions (`sin`, `cos`, `atan2`, ...) come from the platform maths library
//! and may differ in the last bit between OSes. Geometry hashing therefore
//! goes through [`StableHasher::write_f64_quantized`] where trig is involved,
//! and the kernel bake-off (ADR-0004) measures cross-OS determinism directly.
//! If last-bit differences show up, swap in a portable `libm` here, in one
//! place.
//!
//! ## Must never
//!
//! - Panic on degenerate input: normalising a zero vector returns `None`.
//! - Depend on any crate above the foundation layer.
#![forbid(unsafe_code)]

mod bbox;
mod hash;
mod tolerance;
mod transform;
mod vec;

pub use bbox::Aabb;
pub use hash::StableHasher;
pub use tolerance::{ToleranceError, Tolerances};
pub use transform::{Csys, CsysError, Mat3, Transform};
pub use vec::{Point3, UnitVec3, Vec3};
