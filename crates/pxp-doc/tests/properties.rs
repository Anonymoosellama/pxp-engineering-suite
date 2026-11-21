// SPDX-License-Identifier: LGPL-2.1-or-later
//! Property tests for the document core: any sequence of edits undoes back to
//! the empty document and redoes to the same state; failed transactions
//! never change anything.

#![allow(clippy::unwrap_used)] // test-only crate

use proptest::prelude::*;
use pxp_doc::{Document, Object, SequentialIdSource};

#[derive(Debug, Clone)]
enum Edit {
    Set(usize, i32),
    Derive(usize, usize),
    Bad(usize),
}

fn edit() -> impl Strategy<Value = Edit> {
    prop_oneof![
        (0usize..6, -1000i32..1000).prop_map(|(n, v)| Edit::Set(n, v)),
        (0usize..6, 0usize..6).prop_map(|(a, b)| Edit::Derive(a, b)),
        (0usize..6).prop_map(Edit::Bad),
    ]
}

fn apply(doc: &mut Document, e: &Edit) {
    // Errors (unknown names, cycles, dimension errors) are expected for some
    // random edits; the property is that they leave no trace.
    let before = doc.generation();
    let r = doc.transact("edit", |tx| match e {
        Edit::Set(n, v) => tx
            .set_expression(&format!("e{n}"), &format!("{v}mm"))
            .map(|_| ()),
        Edit::Derive(a, b) => tx
            .set_expression(&format!("e{a}"), &format!("e{b} * 2"))
            .map(|_| ()),
        Edit::Bad(n) => tx
            .set_expression(&format!("e{n}"), "1mm + 1deg")
            .map(|_| ()),
    });
    if r.is_err() {
        assert_eq!(
            doc.generation(),
            before,
            "failed transaction changed the document"
        );
    }
}

fn contents(doc: &Document) -> Vec<Object> {
    doc.snapshot().objects().cloned().collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn undo_all_then_redo_all_is_identity(edits in proptest::collection::vec(edit(), 0..40)) {
        let mut doc = Document::with_id_source(Box::new(SequentialIdSource::new(1)));
        for e in &edits {
            apply(&mut doc, e);
        }
        let end = contents(&doc);
        let mut steps = 0;
        while doc.undo().is_ok() {
            steps += 1;
        }
        prop_assert!(doc.snapshot().is_empty(), "undoing everything returns to the empty document");
        for _ in 0..steps {
            doc.redo().unwrap();
        }
        prop_assert_eq!(contents(&doc), end);
    }
}
