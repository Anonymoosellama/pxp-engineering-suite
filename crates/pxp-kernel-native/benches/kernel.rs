// SPDX-License-Identifier: LGPL-2.1-or-later
//! Kernel benchmarks behind the Stage 0 "recompute time" budget
//! (`docs/design/performance-budgets.md`): a primitive, a boolean that
//! drills a block, and mass properties, through `KernelHost` as a feature
//! recompute calls them.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use criterion::{Criterion, criterion_group, criterion_main};
use pxp_kernel::{BlockSpec, BooleanOp, CylinderSpec, KernelHost};
use pxp_kernel_native::NativeKernel;
use pxp_math::{Csys, Point3, UnitVec3};
use pxp_units::Length;
use std::hint::black_box;

fn spec() -> (BlockSpec, CylinderSpec) {
    (
        BlockSpec {
            csys: Csys::ABSOLUTE,
            x: Length::from_mm(20.0),
            y: Length::from_mm(20.0),
            z: Length::from_mm(10.0),
        },
        CylinderSpec {
            base: Point3::new(10.0, 10.0, -1.0),
            axis: UnitVec3::Z,
            diameter: Length::from_mm(8.0),
            height: Length::from_mm(12.0),
        },
    )
}

fn kernel(c: &mut Criterion) {
    let host = KernelHost::in_process(Box::new(NativeKernel::new()));
    let (block, hole) = spec();
    c.bench_function("kernel/native/block", |b| {
        b.iter(|| {
            let r = host.op(|k| k.block(&block)).unwrap();
            host.call(|k| {
                k.release(r.shape);
                Ok(())
            })
            .unwrap();
        });
    });
    let a = host.op(|k| k.block(&block)).unwrap().shape;
    let t = host.op(|k| k.cylinder(&hole)).unwrap().shape;
    c.bench_function("kernel/native/drill-block", |b| {
        b.iter(|| {
            let r = host.op(|k| k.boolean(BooleanOp::Subtract, a, t)).unwrap();
            host.call(|k| {
                k.release(r.shape);
                Ok(())
            })
            .unwrap();
        });
    });
    let drilled = host
        .op(|k| k.boolean(BooleanOp::Subtract, a, t))
        .unwrap()
        .shape;
    c.bench_function("kernel/native/mass-properties", |b| {
        b.iter(|| black_box(host.call(|k| k.mass_properties(drilled)).unwrap()));
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(20);
    targets = kernel
}
criterion_main!(benches);
