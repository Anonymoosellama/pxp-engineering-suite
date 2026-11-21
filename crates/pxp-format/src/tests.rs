// SPDX-License-Identifier: LGPL-2.1-or-later
//! Tests of the file format: round trips, locking, recovery, backups,
//! damaged and foreign files, schema versions and migrations.

use super::*;
use pxp_compute::CancelToken;
use pxp_doc::{
    AppId, ExpressionsOnly, NewObject, Object, ObjectKind, ObjectState, SequentialIdSource, Value,
};
use pxp_units::{Dim, Quantity};
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A fresh directory per test (tests run in parallel).
fn dir(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("pxp-format-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn doc() -> Document {
    Document::with_id_source(Box::new(SequentialIdSource::new(1_700_000_000_000)))
}

fn sample_doc() -> Document {
    let mut d = doc();
    d.transact("Build", |tx| {
        let w = tx.set_expression("width", "4in")?;
        tx.set_expression("p1", "width * 2")?;
        tx.create(
            NewObject::new(
                ObjectKind::from_static("modeling.block"),
                AppId::from_static("pxp.modeling"),
            )
            .name("Block(1)")
            .dep(w, "param")
            .prop("note", Value::Text("héllo".into()))
            .prop(
                "len",
                Value::Quantity(Quantity::from_internal(12.5, Dim::LENGTH)),
            )
            .payload(vec![1, 2, 3]),
        )?;
        // A kind from a plugin this build does not know: must survive untouched.
        tx.create(
            NewObject::new(
                ObjectKind::new("someplugin.widget")?,
                AppId::new("someplugin.app")?,
            )
            .payload((0..=255).collect())
            .state(ObjectState::Failed)
            .diagnostic(Some("plugin missing".into())),
        )?;
        Ok(())
    })
    .unwrap();
    d
}

fn objects(d: &Document) -> Vec<Object> {
    d.snapshot().objects().cloned().collect()
}

#[test]
fn create_then_open_round_trips_everything() {
    let dir = dir("roundtrip");
    let path = dir.join("part.pxp");
    let d = sample_doc();
    let file = PartFile::create(&path, PartMeta::inch(), &d).unwrap();
    drop(file);
    let opened = PartFile::open(&path).unwrap();
    assert_eq!(
        objects(&opened.document),
        objects(&d),
        "every field of every object"
    );
    assert_eq!(opened.file.meta(), PartMeta::inch());
    assert_eq!(opened.schema, SchemaStatus::Current);
    assert!(opened.recovery.is_none());
    assert!(!opened.document.can_undo());
}

#[test]
fn large_payloads_are_chunked_and_restored() {
    let dir = dir("chunks");
    let path = dir.join("big.pxp");
    let mut d = doc();
    let big: Vec<u8> = (0..(CHUNK_BYTES * 2 + 12_345))
        .map(|i| u8::try_from(i % 251).unwrap())
        .collect();
    let id = d
        .transact("Big", |tx| {
            tx.create(
                NewObject::new(
                    ObjectKind::from_static("cae.scan"),
                    AppId::from_static("pxp.cae"),
                )
                .payload(big.clone()),
            )
        })
        .unwrap();
    drop(PartFile::create(&path, PartMeta::millimetre(), &d).unwrap());
    let opened = PartFile::open(&path).unwrap();
    assert_eq!(opened.document.snapshot().get(id).unwrap().payload, big);
}

#[test]
fn second_session_is_locked_out_but_can_view() {
    let dir = dir("lock");
    let path = dir.join("part.pxp");
    let _first = PartFile::create(&path, PartMeta::millimetre(), &sample_doc()).unwrap();
    assert!(matches!(
        PartFile::open(&path),
        Err(FormatError::Platform(PlatformError::Locked { .. }))
    ));
    let view = PartFile::open_read_only(&path).unwrap();
    assert!(view.file.read_only_reason().is_some());
}

#[test]
fn crash_recovery_restores_the_last_committed_action() {
    let dir = dir("recovery");
    let path = dir.join("part.pxp");
    let mut d = sample_doc();
    let mut file = PartFile::create(&path, PartMeta::millimetre(), &d).unwrap();
    let rx = d.subscribe();
    d.transact("Edit width", |tx| tx.set_expression("width", "5in"))
        .unwrap();
    d.recompute(&ExpressionsOnly, &CancelToken::new()).unwrap();
    d.transact("Add depth", |tx| tx.set_expression("depth", "7mm"))
        .unwrap();
    for change in rx.try_iter() {
        file.record(&d.snapshot(), &change).unwrap();
    }
    drop(file); // the crash: no save
    let mut opened = PartFile::open(&path).unwrap();
    let summary = opened.recovery.clone().unwrap();
    assert_eq!(summary.transactions, 3, "edit, its recompute, and the add");
    assert_eq!(summary.last_label, "Add depth");
    assert!(
        opened.document.snapshot().expression("depth").is_none(),
        "saved state first"
    );
    let recovered = opened.file.recovered_document().unwrap();
    let snap = recovered.snapshot();
    assert_eq!(
        snap.expression_value("depth"),
        Some(Quantity::from_internal(7.0, Dim::LENGTH))
    );
    let p1 = snap.expression_value("p1").unwrap().value();
    assert!((p1 - 254.0).abs() < 1e-9, "recomputed value journalled too");
    opened.file.discard_recovery().unwrap();
    drop(opened);
    assert!(PartFile::open(&path).unwrap().recovery.is_none());
}

#[test]
fn save_clears_the_journal_and_keeps_backups() {
    let dir = dir("backups");
    let path = dir.join("part.pxp");
    let mut d = sample_doc();
    let mut file = PartFile::create(&path, PartMeta::millimetre(), &d).unwrap();
    for i in 1..=4 {
        let rx = d.subscribe();
        d.transact(&format!("Edit {i}"), |tx| {
            tx.set_expression("width", &format!("{i}mm"))
        })
        .unwrap();
        for change in rx.try_iter() {
            file.record(&d.snapshot(), &change).unwrap();
        }
        let report = file.save(&d, PartMeta::millimetre()).unwrap();
        assert_eq!(report.objects, 4);
    }
    assert_eq!(file.backups().len(), BACKUPS_KEPT);
    drop(file);
    let opened = PartFile::open(&path).unwrap();
    assert!(opened.recovery.is_none(), "saved: nothing to recover");
    assert_eq!(
        opened.document.snapshot().expression_value("width"),
        Some(Quantity::from_internal(4.0, Dim::LENGTH))
    );
    let bak1 = PartFile::open_read_only(&sidecar(&path, ".bak1")).unwrap();
    assert_eq!(
        bak1.document.snapshot().expression_value("width"),
        Some(Quantity::from_internal(3.0, Dim::LENGTH))
    );
}

#[test]
fn foreign_and_damaged_files_are_refused_without_panicking() {
    let dir = dir("damaged");
    let garbage = dir.join("garbage.pxp");
    std::fs::write(&garbage, b"this is not a database at all, just text").unwrap();
    assert!(matches!(
        PartFile::open_read_only(&garbage),
        Err(FormatError::NotAPxpFile(_))
    ));

    let other = dir.join("other.pxp");
    let c = Connection::open(&other).unwrap();
    c.execute_batch("CREATE TABLE t (x); INSERT INTO t VALUES (1);")
        .unwrap();
    drop(c);
    assert!(matches!(
        PartFile::open_read_only(&other),
        Err(FormatError::NotAPxpFile(_))
    ));

    let good = dir.join("good.pxp");
    drop(PartFile::create(&good, PartMeta::millimetre(), &sample_doc()).unwrap());
    let bytes = std::fs::read(&good).unwrap();
    // Truncations and byte flips: always an error or a valid document, never a panic.
    for cut in [0, 50, 100, 512, bytes.len() / 2, bytes.len() - 1] {
        let p = dir.join(format!("cut{cut}.pxp"));
        std::fs::write(&p, &bytes[..cut]).unwrap();
        let _ = PartFile::open_read_only(&p);
    }
    for (i, at) in (100..bytes.len()).step_by(997).enumerate() {
        let mut b = bytes.clone();
        b[at] ^= 0x5A;
        let p = dir.join(format!("flip{i}.pxp"));
        std::fs::write(&p, &b).unwrap();
        let _ = PartFile::open_read_only(&p);
    }
}

#[test]
fn inconsistent_contents_are_reported() {
    let dir = dir("dangling");
    let path = dir.join("part.pxp");
    drop(PartFile::create(&path, PartMeta::millimetre(), &sample_doc()).unwrap());
    let c = Connection::open(&path).unwrap();
    c.execute("DELETE FROM objects WHERE name = 'width'", [])
        .unwrap();
    drop(c);
    assert!(matches!(
        PartFile::open_read_only(&path),
        Err(FormatError::InvalidContent(_))
    ));
}

#[test]
fn newer_files_open_read_only_and_refuse_writes() {
    let dir = dir("newer");
    let path = dir.join("part.pxp");
    drop(PartFile::create(&path, PartMeta::millimetre(), &sample_doc()).unwrap());
    let c = Connection::open(&path).unwrap();
    c.pragma_update(None, "user_version", SCHEMA_VERSION + 5)
        .unwrap();
    drop(c);
    let mut opened = PartFile::open(&path).unwrap();
    assert_eq!(
        opened.schema,
        SchemaStatus::Newer {
            found: SCHEMA_VERSION + 5
        }
    );
    assert!(
        opened
            .file
            .read_only_reason()
            .unwrap()
            .contains("newer version")
    );
    let d = opened.document;
    assert!(matches!(
        opened.file.save(&d, PartMeta::millimetre()),
        Err(FormatError::ReadOnly(_))
    ));
}

/// A synthetic "version 0" schema (version 1 without `payload_chunks`) and
/// the step that upgrades it, to exercise the migration framework.
fn v0_to_v1(tx: &rusqlite::Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch("ALTER TABLE objects ADD COLUMN payload_chunks INTEGER NOT NULL DEFAULT 0;")
}

#[test]
fn older_files_are_migrated_in_memory() {
    let dir = dir("migrate");
    let path = dir.join("part.pxp");
    drop(PartFile::create(&path, PartMeta::millimetre(), &sample_doc()).unwrap());
    let c = Connection::open(&path).unwrap();
    c.execute_batch("ALTER TABLE objects DROP COLUMN payload_chunks;")
        .unwrap();
    c.pragma_update(None, "user_version", 0).unwrap();
    drop(c);
    let steps = [Migration {
        from: 0,
        describe: "add payload_chunks",
        apply: v0_to_v1,
    }];
    let opened = open_with(&path, &steps, true).unwrap();
    assert_eq!(
        opened.schema,
        SchemaStatus::Migrated {
            from: 0,
            steps: vec!["add payload_chunks"]
        }
    );
    assert_eq!(opened.document.snapshot().len(), 4);
    let on_disk: i32 = Connection::open(&path)
        .unwrap()
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(on_disk, 0, "the file itself is only upgraded by a save");
    assert!(matches!(
        open_with(&path, &[], true),
        Err(FormatError::NoMigration(0))
    ));
}

#[test]
fn save_as_moves_lock_and_journal() {
    let dir = dir("saveas");
    let (a, b) = (dir.join("a.pxp"), dir.join("b.pxp"));
    let d = sample_doc();
    let mut file = PartFile::create(&a, PartMeta::millimetre(), &d).unwrap();
    file.save_as(&b, &d, PartMeta::millimetre()).unwrap();
    assert_eq!(file.path(), b.as_path());
    assert!(!sidecar(&a, ".journal").exists(), "old journal removed");
    assert!(
        matches!(PartFile::open(&b), Err(FormatError::Platform(_))),
        "new path held"
    );
    assert!(PartFile::open(&a).is_ok(), "old path released");
}

#[test]
fn create_refuses_to_overwrite() {
    let dir = dir("exists");
    let path = dir.join("part.pxp");
    drop(PartFile::create(&path, PartMeta::millimetre(), &doc()).unwrap());
    assert!(matches!(
        PartFile::create(&path, PartMeta::millimetre(), &doc()),
        Err(FormatError::AlreadyExists(_))
    ));
}

#[test]
fn identical_documents_write_identical_object_rows() {
    let dir = dir("determinism");
    let rows = |name: &str| -> Vec<(Vec<u8>, Vec<u8>, Vec<u8>)> {
        let path = dir.join(name);
        drop(PartFile::create(&path, PartMeta::millimetre(), &sample_doc()).unwrap());
        let c = Connection::open(&path).unwrap();
        let mut s = c
            .prepare("SELECT id, props, payload FROM objects ORDER BY id")
            .unwrap();
        s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(rows("one.pxp"), rows("two.pxp"));
}
