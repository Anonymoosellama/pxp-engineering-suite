// SPDX-License-Identifier: LGPL-2.1-or-later
//! Contract tests with a test-only mock backend. The mock is compiled only
//! for tests and is not a geometry kernel: it fabricates topology lists to
//! exercise the host's contract checks.

use super::*;
use pxp_math::Vec3;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, PartialEq)]
enum Behaviour {
    Honest,
    ForgetsAFace,
    Panics,
}

struct MockKernel {
    behaviour: Behaviour,
    next: AtomicU64,
}

impl MockKernel {
    fn new(behaviour: Behaviour) -> Self {
        Self {
            behaviour,
            next: AtomicU64::new(1),
        }
    }
    fn face(i: u32) -> TopoId {
        TopoId {
            kind: TopoKind::Face,
            index: i,
        }
    }
}

const MOCK: BackendId = BackendId("mock");

impl GeometryKernel for MockKernel {
    fn backend_id(&self) -> BackendId {
        MOCK
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            ops: ["block"].into_iter().collect(),
        }
    }
    fn block(&self, spec: &BlockSpec) -> Result<OpResult, KernelError> {
        spec.validate()?;
        assert!(self.behaviour != Behaviour::Panics, "mock backend exploded");
        let records = (0..6_u32)
            .filter(|&i| !(self.behaviour == Behaviour::ForgetsAFace && i == 5))
            .zip(BlockSpec::FACE_ROLES)
            .map(|(i, role)| HistoryRecord {
                output: Self::face(i),
                relation: Relation::Generated,
                sources: vec![HistorySource::Role(role)],
            })
            .collect();
        let shape = ShapeHandle {
            backend: MOCK,
            id: self.next.fetch_add(1, Ordering::Relaxed),
        };
        Ok(OpResult {
            shape,
            history: TopoHistory {
                records,
                deleted: vec![],
            },
            warnings: vec![],
        })
    }
    fn cylinder(&self, _: &CylinderSpec) -> Result<OpResult, KernelError> {
        Err(KernelError::Unsupported {
            op: "cylinder",
            reason: "mock".into(),
        })
    }
    fn cone(&self, _: &ConeSpec) -> Result<OpResult, KernelError> {
        Err(KernelError::Unsupported {
            op: "cone",
            reason: "mock".into(),
        })
    }
    fn sphere(&self, _: &SphereSpec) -> Result<OpResult, KernelError> {
        Err(KernelError::Unsupported {
            op: "sphere",
            reason: "mock".into(),
        })
    }
    fn boolean(
        &self,
        _: BooleanOp,
        _: ShapeHandle,
        _: ShapeHandle,
    ) -> Result<OpResult, KernelError> {
        Err(KernelError::Unsupported {
            op: "boolean",
            reason: "mock".into(),
        })
    }
    fn topology(&self, shape: ShapeHandle) -> Result<Vec<TopoId>, KernelError> {
        if shape.backend != MOCK {
            return Err(KernelError::UnknownShape(shape));
        }
        Ok((0..6).map(Self::face).collect())
    }
    fn body_kind(&self, _: ShapeHandle) -> Result<BodyKind, KernelError> {
        Ok(BodyKind::Solid)
    }
    fn mass_properties(&self, shape: ShapeHandle) -> Result<MassProperties, KernelError> {
        Err(KernelError::Unsupported {
            op: "mass_properties",
            reason: format!("mock {}", shape.id),
        })
    }
    fn geometry_hash(&self, shape: ShapeHandle, _: Length) -> Result<u64, KernelError> {
        Ok(shape.id)
    }
    fn release(&self, _: ShapeHandle) {}
}

fn block() -> BlockSpec {
    BlockSpec {
        csys: Csys::ABSOLUTE,
        x: Length::from_mm(10.0),
        y: Length::from_mm(20.0),
        z: Length::from_mm(30.0),
    }
}

#[test]
fn honest_backend_passes_contract() {
    let host = KernelHost::in_process(Box::new(MockKernel::new(Behaviour::Honest)));
    let r = host.op(|k| k.block(&block())).unwrap();
    let top = HistorySource::Role("block.z_max");
    assert_eq!(
        r.history.outputs_from(&top).collect::<Vec<_>>(),
        vec![MockKernel::face(5)]
    );
    assert_eq!(r.history.sources_of(MockKernel::face(0)).count(), 1);
}

#[test]
fn incomplete_history_is_rejected_before_storage() {
    let host = KernelHost::in_process(Box::new(MockKernel::new(Behaviour::ForgetsAFace)));
    match host.op(|k| k.block(&block())) {
        Err(KernelError::IncompleteHistory(missing)) => {
            assert_eq!(missing, vec![MockKernel::face(5)]);
        }
        other => panic!("expected IncompleteHistory, got {other:?}"),
    }
}

#[test]
fn backend_panic_is_contained() {
    let host = KernelHost::in_process(Box::new(MockKernel::new(Behaviour::Panics)));
    match host.op(|k| k.block(&block())) {
        Err(KernelError::BackendPanicked(m)) => assert!(m.contains("exploded")),
        other => panic!("expected BackendPanicked, got {other:?}"),
    }
}

#[test]
fn unsupported_ops_are_typed_refusals() {
    let host = KernelHost::in_process(Box::new(MockKernel::new(Behaviour::Honest)));
    assert!(host.capabilities().supports("block"));
    assert!(!host.capabilities().supports("cylinder"));
    let spec = CylinderSpec {
        base: Point3::ORIGIN,
        axis: UnitVec3::Z,
        diameter: Length::from_mm(5.0),
        height: Length::from_mm(5.0),
    };
    assert!(matches!(
        host.op(|k| k.cylinder(&spec)),
        Err(KernelError::Unsupported { .. })
    ));
}

#[test]
fn spec_validation() {
    let mut b = block();
    b.y = Length::ZERO;
    assert!(matches!(b.validate(), Err(KernelError::InvalidInput(_))));
    let cone = ConeSpec {
        base: Point3::ORIGIN,
        axis: Vec3::new(0.0, 0.0, 1.0).normalized().unwrap(),
        base_diameter: Length::from_mm(10.0),
        top_diameter: Length::from_mm(10.0),
        height: Length::from_mm(5.0),
    };
    assert!(cone.validate().is_err(), "equal diameters is a cylinder");
    let pointed = ConeSpec {
        top_diameter: Length::ZERO,
        ..cone
    };
    assert!(pointed.validate().is_ok());
    let s = SphereSpec {
        center: Point3::new(f64::NAN, 0.0, 0.0),
        diameter: Length::from_mm(1.0),
    };
    assert!(s.validate().is_err());
}

#[test]
fn check_complete_ignores_shells_and_bodies() {
    let h = TopoHistory::default();
    let body = TopoId {
        kind: TopoKind::Body,
        index: 0,
    };
    assert!(h.check_complete([body]).is_ok());
    assert!(h.check_complete([MockKernel::face(0)]).is_err());
}
