// SPDX-License-Identifier: LGPL-2.1-or-later
//! File benchmarks behind the Stage 0 "open time" budget
//! (`docs/design/performance-budgets.md`): saving and opening a part with
//! 10 000 objects.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use criterion::{Criterion, criterion_group, criterion_main};
use pxp_doc::Document;
use pxp_format::{PartFile, PartMeta};
use std::hint::black_box;
use std::path::PathBuf;

fn document(n: usize) -> Document {
    let mut doc = Document::new();
    doc.transact("setup", |tx| {
        for i in 0..n {
            tx.set_expression(&format!("p{i}"), &format!("{i}mm"))?;
        }
        Ok(())
    })
    .unwrap();
    doc
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pxp-bench-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("part.pxp")
}

fn save_and_open(c: &mut Criterion) {
    let doc = document(10_000);
    let path = scratch("10k");
    let file = PartFile::create(&path, PartMeta::millimetre(), &doc).unwrap();
    file.close().unwrap();
    c.bench_function("file/open-10k-objects", |b| {
        b.iter(|| {
            let opened = PartFile::open_read_only(&path).unwrap();
            black_box(opened.document.generation())
        });
    });
    c.bench_function("file/save-10k-objects", |b| {
        let mut file = PartFile::open(&path).unwrap().file;
        b.iter(|| black_box(file.save(&doc, PartMeta::millimetre()).unwrap()));
    });
}

criterion_group!(benches, save_and_open);
criterion_main!(benches);
