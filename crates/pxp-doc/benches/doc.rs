// SPDX-License-Identifier: LGPL-2.1-or-later
//! Document-core benchmarks behind the Stage 0 budgets
//! (`docs/design/performance-budgets.md`): the cost of one edit in a large
//! document, of undo and redo, and of recomputing a long dependency chain.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use pxp_compute::CancelToken;
use pxp_doc::{Document, ExpressionsOnly};
use std::hint::black_box;

/// A document with `n` independent expressions `p0 … p(n-1)`.
fn wide(n: usize) -> Document {
    let mut doc = Document::new();
    doc.transact("setup", |tx| {
        for i in 0..n {
            tx.set_expression(&format!("p{i}"), "1mm")?;
        }
        Ok(())
    })
    .unwrap();
    doc
}

fn edit_in_large_document(c: &mut Criterion) {
    let mut doc = wide(100_000);
    let mut flip = false;
    c.bench_function("doc/edit-one-of-100k", |b| {
        b.iter(|| {
            flip = !flip;
            let formula = if flip { "2mm" } else { "3mm" };
            doc.transact("edit", |tx| tx.set_expression("p50000", formula))
                .unwrap();
            black_box(doc.generation())
        });
    });
}

fn undo_redo(c: &mut Criterion) {
    c.bench_function("doc/undo-redo-100-steps", |b| {
        b.iter_batched(
            || {
                let mut doc = wide(1_000);
                for i in 0..100 {
                    doc.transact("edit", |tx| tx.set_expression(&format!("p{i}"), "5mm"))
                        .unwrap();
                }
                doc
            },
            |mut doc| {
                for _ in 0..100 {
                    doc.undo().unwrap();
                }
                for _ in 0..100 {
                    doc.redo().unwrap();
                }
                doc
            },
            BatchSize::LargeInput,
        );
    });
}

fn recompute_chain(c: &mut Criterion) {
    let mut doc = Document::new();
    doc.transact("chain", |tx| {
        tx.set_expression("q0", "1mm")?;
        for i in 1..1_000 {
            tx.set_expression(&format!("q{i}"), &format!("q{} + 1mm", i - 1))?;
        }
        Ok(())
    })
    .unwrap();
    doc.recompute(&ExpressionsOnly, &CancelToken::new())
        .unwrap();
    let mut flip = false;
    c.bench_function("doc/recompute-chain-1000", |b| {
        b.iter(|| {
            flip = !flip;
            let formula = if flip { "2mm" } else { "1mm" };
            doc.transact("edit", |tx| tx.set_expression("q0", formula))
                .unwrap();
            doc.recompute(&ExpressionsOnly, &CancelToken::new())
                .unwrap();
            black_box(doc.snapshot().expression_value("q999"))
        });
    });
}

criterion_group!(benches, edit_in_large_document, undo_redo, recompute_chain);
criterion_main!(benches);
