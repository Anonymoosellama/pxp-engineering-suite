// SPDX-License-Identifier: LGPL-2.1-or-later
//! Integration tests for the document core: transactions, rollback, undo,
//! dependency propagation and recompute. These cover the headless parts of
//! the Stage 0 verification checklist ("100 undo and redo steps behave
//! correctly"; panics roll back and the session continues).

// This file is a test-only crate that is never shipped. Its helpers (the test
// recompute handler, model builders) are not `#[test]` functions, so clippy's
// test exemptions in clippy.toml do not reach them; allow the same lints here.
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::manual_assert,
    clippy::float_cmp
)]

use pxp_compute::CancelToken;
use pxp_doc::{
    AppId, ChangeOrigin, DocError, Document, ExpressionsOnly, NewObject, Object, ObjectId,
    ObjectKind, ObjectState, RecomputeHandler, RecomputeOutput, SequentialIdSource, Snapshot,
    UpdatePolicy, Value,
};
use pxp_units::{Dim, Quantity};
use std::collections::BTreeMap;

const TEST_APP: AppId = AppId::from_static("pxp.test");
const DOUBLE: ObjectKind = ObjectKind::from_static("test.double");
const CHAIN: ObjectKind = ObjectKind::from_static("test.chain");
const MANUAL: ObjectKind = ObjectKind::from_static("test.manual");

fn doc() -> Document {
    Document::with_id_source(Box::new(SequentialIdSource::new(1_700_000_000_000)))
}

fn mm(v: f64) -> Quantity {
    Quantity::from_internal(v, Dim::LENGTH)
}

fn result_of(snap: &Snapshot, id: ObjectId) -> Option<f64> {
    match snap.get(id)?.prop("result")? {
        Value::Quantity(q) => Some(q.value()),
        _ => None,
    }
}

/// Test application kinds:
/// - `test.double`: result = 2 × the value of the expression it depends on
///   (role "param"); fails if its `fail` property is true.
/// - `test.chain`:  result = 1 mm + the result of the object it depends on.
/// - `test.manual`: manual update policy (like a CAM toolpath).
struct TestHandler;

impl RecomputeHandler for TestHandler {
    fn policy(&self, kind: &ObjectKind) -> UpdatePolicy {
        if *kind == MANUAL {
            UpdatePolicy::Manual
        } else {
            UpdatePolicy::Immediate
        }
    }

    fn recompute(&self, inputs: &Snapshot, object: &Object) -> Result<RecomputeOutput, String> {
        if object.prop("fail") == Some(&Value::Bool(true)) {
            return Err("deliberate failure".into());
        }
        if object.prop("panic") == Some(&Value::Bool(true)) {
            panic!("deliberate panic in handler");
        }
        let input = object.deps.iter().next().ok_or("no input")?.on;
        let input_obj = inputs.get(input).ok_or("input missing")?;
        let base = if input_obj.kind == ObjectKind::EXPRESSION {
            match input_obj.prop("value") {
                Some(Value::Quantity(q)) => q.value() * 2.0,
                _ => return Err("expression has no value".into()),
            }
        } else {
            result_of(inputs, input).ok_or("input has no result")? + 1.0
        };
        let mut props = BTreeMap::new();
        props.insert("result".to_owned(), Value::Quantity(mm(base)));
        Ok(RecomputeOutput {
            props,
            payload: None,
            warning: None,
        })
    }
}

/// width (expr) <- double <- chain
fn three_level_model(doc: &mut Document) -> (ObjectId, ObjectId, ObjectId) {
    doc.transact("Build", |tx| {
        let width = tx.set_expression("width", "10mm")?;
        let double = tx.create(
            NewObject::new(DOUBLE, TEST_APP)
                .name("Double")
                .dep(width, "param"),
        )?;
        let chain = tx.create(
            NewObject::new(CHAIN, TEST_APP)
                .name("Chain")
                .dep(double, "input"),
        )?;
        Ok((width, double, chain))
    })
    .unwrap()
}

#[test]
fn error_in_transaction_rolls_back_everything() {
    let mut d = doc();
    let before = d.generation();
    let r: Result<(), DocError> = d.transact("Bad", |tx| {
        tx.set_expression("a", "1mm")?;
        tx.set_expression("b", "a + 1deg")?; // dimension error
        Ok(())
    });
    assert!(matches!(r, Err(DocError::Expression { .. })));
    assert_eq!(d.generation(), before, "nothing committed");
    assert!(
        d.snapshot().expression("a").is_none(),
        "first edit rolled back too"
    );
    assert!(!d.can_undo());
}

#[test]
fn panic_in_transaction_rolls_back_and_session_continues() {
    let mut d = doc();
    d.transact("Ok", |tx| tx.set_expression("a", "1mm"))
        .unwrap();
    let r: Result<(), DocError> = d.transact("Crash", |tx| {
        tx.set_expression("b", "2mm")?;
        panic!("simulated bug in a command");
    });
    match r {
        Err(DocError::Panicked { label, message }) => {
            assert_eq!(label, "Crash");
            assert!(message.contains("simulated bug"));
        }
        other => panic!("expected Panicked, got {other:?}"),
    }
    let snap = d.snapshot();
    assert!(snap.expression("a").is_some());
    assert!(snap.expression("b").is_none());
    // The session continues normally.
    d.transact("After", |tx| tx.set_expression("c", "3mm"))
        .unwrap();
    assert_eq!(d.undo_labels(), vec!["After", "Ok"]);
}

#[test]
fn one_hundred_undo_and_redo_steps() {
    let mut d = doc();
    for i in 0..100 {
        d.transact(&format!("Step {i}"), |tx| {
            tx.set_expression(&format!("p{i}"), &format!("{i}mm"))
        })
        .unwrap();
    }
    let full: Vec<(String, f64)> = d
        .snapshot()
        .objects()
        .map(|o| {
            (
                o.name.clone().unwrap_or_default(),
                match o.prop("value") {
                    Some(Value::Quantity(q)) => q.value(),
                    _ => f64::NAN,
                },
            )
        })
        .collect();
    assert_eq!(full.len(), 100);
    for i in (0..100).rev() {
        assert_eq!(d.undo().unwrap(), format!("Step {i}"));
        assert_eq!(d.snapshot().len(), i);
    }
    assert!(matches!(d.undo(), Err(DocError::NothingToUndo)));
    for i in 0..100 {
        assert_eq!(d.redo().unwrap(), format!("Step {i}"));
    }
    assert!(matches!(d.redo(), Err(DocError::NothingToRedo)));
    let again: Vec<(String, f64)> = d
        .snapshot()
        .objects()
        .map(|o| {
            (
                o.name.clone().unwrap_or_default(),
                match o.prop("value") {
                    Some(Value::Quantity(q)) => q.value(),
                    _ => f64::NAN,
                },
            )
        })
        .collect();
    assert_eq!(full, again, "redo restores exactly the same objects");
}

#[test]
fn new_edit_after_undo_clears_redo() {
    let mut d = doc();
    d.transact("A", |tx| tx.set_expression("a", "1")).unwrap();
    d.transact("B", |tx| tx.set_expression("b", "2")).unwrap();
    d.undo().unwrap();
    assert!(d.can_redo());
    d.transact("C", |tx| tx.set_expression("c", "3")).unwrap();
    assert!(!d.can_redo());
}

#[test]
fn clearing_undo_keeps_the_state_and_forgets_the_steps() {
    let mut d = doc();
    d.transact("A", |tx| tx.set_expression("a", "1")).unwrap();
    d.transact("B", |tx| tx.set_expression("b", "2")).unwrap();
    d.undo().unwrap();
    d.clear_undo();
    assert!(!d.can_undo() && !d.can_redo());
    assert_eq!(d.undo_memory_used(), 0);
    assert!(d.snapshot().expression("a").is_some(), "the state is kept");
    assert!(d.snapshot().expression("b").is_none());
    d.transact("C", |tx| tx.set_expression("c", "3")).unwrap();
    assert_eq!(d.undo().unwrap(), "C", "new steps record as before");
}

#[test]
fn edits_propagate_out_of_date_and_recompute_restores() {
    let mut d = doc();
    let (_, double, chain) = three_level_model(&mut d);
    let report = d.recompute(&TestHandler, &CancelToken::new()).unwrap();
    assert!(report.succeeded());
    assert_eq!(report.levels, 2);
    let snap = d.snapshot();
    assert_eq!(result_of(&snap, double), Some(20.0));
    assert_eq!(result_of(&snap, chain), Some(21.0));

    d.transact("Edit width", |tx| tx.set_expression("width", "1in"))
        .unwrap();
    let snap = d.snapshot();
    assert_eq!(snap.get(double).unwrap().state, ObjectState::OutOfDate);
    assert_eq!(snap.get(chain).unwrap().state, ObjectState::OutOfDate);
    assert_eq!(
        snap.expression("width").unwrap().state,
        ObjectState::UpToDate
    );

    d.recompute(&TestHandler, &CancelToken::new()).unwrap();
    let snap = d.snapshot();
    assert_eq!(result_of(&snap, double), Some(50.8));
    assert_eq!(result_of(&snap, chain), Some(51.8));
    assert!(snap.objects().all(|o| o.state == ObjectState::UpToDate));
}

#[test]
fn recompute_merges_into_the_edit_so_one_undo_reverts_both() {
    let mut d = doc();
    let (_, double, _) = three_level_model(&mut d);
    d.recompute(&TestHandler, &CancelToken::new()).unwrap();
    d.transact("Edit width", |tx| tx.set_expression("width", "30mm"))
        .unwrap();
    d.recompute(&TestHandler, &CancelToken::new()).unwrap();
    assert_eq!(result_of(&d.snapshot(), double), Some(60.0));
    assert_eq!(d.undo().unwrap(), "Edit width");
    let snap = d.snapshot();
    assert_eq!(
        result_of(&snap, double),
        Some(20.0),
        "geometry reverted with the edit"
    );
    assert_eq!(snap.get(double).unwrap().state, ObjectState::UpToDate);
}

#[test]
fn failed_recompute_commits_no_results_only_flags() {
    let mut d = doc();
    let (_, double, chain) = three_level_model(&mut d);
    d.recompute(&TestHandler, &CancelToken::new()).unwrap();
    d.transact("Edit", |tx| {
        tx.set_expression("width", "40mm")?;
        tx.update(double, |o| {
            o.props.insert("fail".into(), Value::Bool(true));
        })
    })
    .unwrap();
    let report = d.recompute(&TestHandler, &CancelToken::new()).unwrap();
    assert_eq!(report.failed.len(), 1);
    assert!(report.recomputed.is_empty());
    let snap = d.snapshot();
    let failed = snap.get(double).unwrap();
    assert_eq!(failed.state, ObjectState::Failed);
    assert_eq!(failed.diagnostic.as_deref(), Some("deliberate failure"));
    assert_eq!(
        result_of(&snap, double),
        Some(20.0),
        "old result kept, not a wrong new one"
    );
    assert_eq!(
        snap.get(chain).unwrap().state,
        ObjectState::OutOfDate,
        "downstream untouched"
    );

    // Fix the feature; recompute succeeds.
    d.transact("Fix", |tx| {
        tx.update(double, |o| {
            o.props.remove("fail");
        })
    })
    .unwrap();
    assert!(
        d.recompute(&TestHandler, &CancelToken::new())
            .unwrap()
            .succeeded()
    );
    assert_eq!(result_of(&d.snapshot(), chain), Some(81.0));
}

#[test]
fn panicking_handler_is_contained() {
    let mut d = doc();
    let (_, double, _) = three_level_model(&mut d);
    d.transact("Arm", |tx| {
        tx.update(double, |o| {
            o.props.insert("panic".into(), Value::Bool(true));
        })
    })
    .unwrap();
    let report = d.recompute(&TestHandler, &CancelToken::new()).unwrap();
    assert_eq!(report.failed.len(), 1);
    assert!(report.failed[0].1.contains("deliberate panic"));
    assert_eq!(d.snapshot().get(double).unwrap().state, ObjectState::Failed);
}

#[test]
fn manual_policy_objects_wait_and_block_their_dependants() {
    let mut d = doc();
    let (width, op, after) = d
        .transact("Build", |tx| {
            let width = tx.set_expression("width", "10mm")?;
            let op = tx.create(
                NewObject::new(MANUAL, TEST_APP)
                    .name("Toolpath")
                    .dep(width, "geom"),
            )?;
            let after = tx.create(NewObject::new(CHAIN, TEST_APP).dep(op, "input"))?;
            Ok((width, op, after))
        })
        .unwrap();
    let report = d.recompute(&TestHandler, &CancelToken::new()).unwrap();
    assert!(report.recomputed.is_empty());
    assert_eq!(report.blocked, vec![after]);
    let snap = d.snapshot();
    assert_eq!(
        snap.get(op).unwrap().state,
        ObjectState::OutOfDate,
        "never regenerated silently"
    );
    assert_eq!(snap.get(width).unwrap().state, ObjectState::UpToDate);
}

#[test]
fn cycles_are_rejected() {
    let mut d = doc();
    d.transact("a", |tx| tx.set_expression("a", "1mm")).unwrap();
    d.transact("b", |tx| tx.set_expression("b", "a * 2"))
        .unwrap();
    let r = d.transact("cycle", |tx| tx.set_expression("a", "b + 1mm"));
    assert!(matches!(r, Err(DocError::Cycle { .. })));
    let r = d.transact("self", |tx| tx.set_expression("a", "a + 1mm"));
    assert!(matches!(r, Err(DocError::Cycle { .. })));
}

#[test]
fn unknown_and_duplicate_names() {
    let mut d = doc();
    let r = d.transact("x", |tx| tx.set_expression("x", "nosuch * 2"));
    assert!(matches!(r, Err(DocError::Expression { .. })));
    let r = d.transact("bad", |tx| tx.set_expression("sin", "1"));
    assert!(matches!(r, Err(DocError::InvalidName(_))));
    let a = d.transact("a", |tx| tx.set_expression("a", "1")).unwrap();
    let r = d.transact("dup", |tx| {
        tx.create(NewObject::new(ObjectKind::EXPRESSION, AppId::CORE).name("a"))
    });
    assert!(matches!(r, Err(DocError::DuplicateExpressionName(_))));
    // Redefining by name updates the same object.
    let again = d.transact("a2", |tx| tx.set_expression("a", "2")).unwrap();
    assert_eq!(a, again);
}

#[test]
fn deletes_never_leave_dangling_references() {
    let mut d = doc();
    let (width, double, chain) = three_level_model(&mut d);
    let r = d.transact("Delete width", |tx| tx.delete(width));
    assert!(matches!(r, Err(DocError::HasDependants { .. })));
    let deleted = d
        .transact("Delete all", |tx| tx.delete_with_dependants(width))
        .unwrap();
    assert_eq!(deleted, vec![chain, double, width], "dependants first");
    assert!(d.snapshot().is_empty());
    d.undo().unwrap();
    let snap = d.snapshot();
    assert_eq!(snap.len(), 3);
    assert_eq!(
        snap.dependants(width).collect::<Vec<_>>(),
        vec![double],
        "index restored"
    );
}

#[test]
fn change_bus_reports_commit_undo_and_redo() {
    let mut d = doc();
    let rx = d.subscribe();
    let id = d
        .transact("Add", |tx| tx.set_expression("a", "1mm"))
        .unwrap();
    d.undo().unwrap();
    d.redo().unwrap();
    let sets: Vec<_> = rx.try_iter().collect();
    assert_eq!(sets.len(), 3);
    assert_eq!(sets[0].origin, ChangeOrigin::Commit);
    assert!(sets[0].created.contains(&id));
    assert_eq!(sets[1].origin, ChangeOrigin::Undo);
    assert!(sets[1].deleted.contains(&id));
    assert_eq!(sets[2].origin, ChangeOrigin::Redo);
    assert!(sets[2].created.contains(&id));
    assert!(sets.windows(2).all(|w| w[0].generation < w[1].generation));
}

#[test]
fn no_op_transactions_record_nothing() {
    let mut d = doc();
    let rx = d.subscribe();
    d.transact("Nothing", |_tx| Ok(())).unwrap();
    let a = d.transact("a", |tx| tx.set_expression("a", "1mm")).unwrap();
    d.transact("Same", |tx| tx.update(a, |_o| {})).unwrap();
    assert_eq!(d.undo_labels(), vec!["a"]);
    assert_eq!(rx.try_iter().count(), 1);
}

#[test]
fn undo_memory_cap_drops_oldest_but_keeps_newest() {
    let mut d = doc();
    d.set_undo_memory_cap(1); // absurdly small
    for i in 0..10 {
        d.transact(&format!("S{i}"), |tx| {
            tx.set_expression(&format!("e{i}"), "1")
        })
        .unwrap();
    }
    assert_eq!(d.undo_labels(), vec!["S9"], "newest step always kept");
    d.set_undo_memory_cap(pxp_doc::DEFAULT_UNDO_CAP_BYTES);
}

#[test]
fn snapshots_are_immutable() {
    let mut d = doc();
    d.transact("a", |tx| tx.set_expression("a", "1mm")).unwrap();
    let old = d.snapshot();
    d.transact("a2", |tx| tx.set_expression("a", "5mm"))
        .unwrap();
    assert_eq!(old.expression_value("a"), Some(mm(1.0)));
    assert_eq!(d.snapshot().expression_value("a"), Some(mm(5.0)));
    assert!(old.generation() < d.generation());
}

#[test]
fn unknown_kind_payload_is_preserved_byte_for_byte() {
    let mut d = doc();
    let bytes: Vec<u8> = (0..=255).collect();
    let kind = ObjectKind::new("someplugin.widget").unwrap();
    let id = d
        .transact("Plugin object", |tx| {
            tx.create(
                NewObject::new(kind.clone(), AppId::new("someplugin.app")?).payload(bytes.clone()),
            )
        })
        .unwrap();
    d.transact("Unrelated", |tx| tx.set_expression("z", "1"))
        .unwrap();
    d.undo().unwrap();
    d.redo().unwrap();
    d.recompute(&ExpressionsOnly, &CancelToken::new()).unwrap();
    assert_eq!(d.snapshot().get(id).unwrap().payload, bytes);
}

#[test]
fn wide_parallel_recompute_is_deterministic() {
    let run = || {
        let mut d = doc();
        d.transact("base", |tx| tx.set_expression("base", "1mm"))
            .unwrap();
        d.transact("many", |tx| {
            for i in 0..500 {
                tx.set_expression(&format!("e{i}"), &format!("base * {i}"))?;
            }
            Ok(())
        })
        .unwrap();
        d.transact("edit", |tx| tx.set_expression("base", "2mm"))
            .unwrap();
        let report = d.recompute(&ExpressionsOnly, &CancelToken::new()).unwrap();
        let snap = d.snapshot();
        let values: Vec<f64> = (0..500)
            .map(|i| snap.expression_value(&format!("e{i}")).unwrap().value())
            .collect();
        (report.recomputed, values)
    };
    let (order_a, values_a) = run();
    let (order_b, values_b) = run();
    assert_eq!(order_a.len(), 500);
    assert_eq!(order_a, order_b, "same recompute order every run");
    assert_eq!(values_a, values_b);
    assert!((values_a[250] - 500.0).abs() < 1e-12);
}

#[test]
fn cancelled_recompute_commits_nothing() {
    let mut d = doc();
    three_level_model(&mut d);
    let gen_before = d.generation();
    let cancel = CancelToken::new();
    cancel.cancel();
    assert!(matches!(
        d.recompute(&TestHandler, &cancel),
        Err(DocError::Cancelled)
    ));
    assert_eq!(d.generation(), gen_before);
}

#[test]
fn evaluate_previews_without_changing_anything() {
    let mut d = doc();
    d.transact("w", |tx| tx.set_expression("width", "100mm"))
        .unwrap();
    let g = d.generation();
    let v = d.evaluate("=width/2").unwrap();
    assert_eq!(v, mm(50.0));
    assert_eq!(d.generation(), g);
}

#[test]
fn from_objects_round_trips_a_snapshot() {
    let mut d = doc();
    let (_, double, chain) = three_level_model(&mut d);
    d.recompute(&TestHandler, &CancelToken::new()).unwrap();
    let snap = d.snapshot();
    let copies: Vec<Object> = snap.objects().cloned().collect();
    let loaded = Document::from_objects(copies, Box::new(SequentialIdSource::new(9))).unwrap();
    let l = loaded.snapshot();
    assert_eq!(l.len(), 3);
    assert_eq!(result_of(&l, chain), Some(21.0));
    assert_eq!(
        l.dependants(double).collect::<Vec<_>>(),
        vec![chain],
        "index rebuilt"
    );
    assert_eq!(l.expression_value("width"), Some(mm(10.0)));
    assert!(!loaded.can_undo(), "opening is not an undo step");
}

#[test]
fn from_objects_rejects_corrupt_graphs() {
    let id = |n: u128| ObjectId::from_u128(n);
    let node = |n: u128, dep: Option<u128>| {
        let mut spec = NewObject::new(CHAIN, TEST_APP);
        if let Some(d) = dep {
            spec = spec.dep(id(d), "input");
        }
        Object::restore(id(n), spec)
    };
    let ids = || -> Box<dyn pxp_doc::IdSource> { Box::new(SequentialIdSource::new(1)) };
    // Dangling dependency.
    assert!(matches!(
        Document::from_objects([node(1, Some(99))], ids()),
        Err(DocError::DanglingDependency { .. })
    ));
    // Cycle 1 -> 2 -> 1.
    assert!(matches!(
        Document::from_objects([node(1, Some(2)), node(2, Some(1))], ids()),
        Err(DocError::Cycle { .. })
    ));
    // Duplicate ID.
    assert!(matches!(
        Document::from_objects([node(1, None), node(1, None)], ids()),
        Err(DocError::Corrupt(_))
    ));
    // Duplicate expression names.
    let e = |n: u128| {
        Object::restore(
            id(n),
            NewObject::new(ObjectKind::EXPRESSION, AppId::CORE).name("w"),
        )
    };
    assert!(matches!(
        Document::from_objects([e(1), e(2)], ids()),
        Err(DocError::DuplicateExpressionName(_))
    ));
}
