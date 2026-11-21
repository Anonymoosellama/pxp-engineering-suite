// SPDX-License-Identifier: LGPL-2.1-or-later
//! The Stage 0 checklist item, headless: "New part from mm and inch
//! templates; create a block; save, close, reopen; geometry hash identical."
//!
//! Every `pxp-cli part …` invocation is a separate process that opens the
//! file, changes it, saves and exits, so consecutive commands really do
//! save, close and reopen.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use std::path::{Path, PathBuf};
use std::process::Command;

fn cli(args: &[&str]) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_pxp-cli"))
        .args(args)
        .output()
        .expect("run pxp-cli");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn ok(args: &[&str]) -> String {
    let (success, stdout, stderr) = cli(args);
    assert!(success, "pxp-cli {args:?} failed:\n{stdout}\n{stderr}");
    stdout
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pxp-cli-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The hash `part block` reported when it created the feature.
fn created_hash(stdout: &str) -> String {
    let start = stdout.find("geometry hash ").unwrap() + "geometry hash ".len();
    stdout[start..start + 16].to_owned()
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

#[test]
fn mm_template_block_survives_save_close_reopen() {
    let dir = scratch("mm");
    let part = dir.join("bracket.pxp");
    ok(&["part", "new", s(&part), "--units", "mm"]);
    let created = ok(&["part", "block", s(&part), "--size", "10", "20", "30"]);
    let hash = created_hash(&created);

    // Reopened in a new process: the stored hash is the one computed at
    // creation, and the stored B-rep measures 10 x 20 x 30 mm.
    let info = ok(&["part", "info", s(&part)]);
    assert!(info.contains(&format!("hash {hash}")), "{info}");
    assert!(info.contains("volume 6000.0000 mm^3"), "{info}");
    assert!(info.contains("area 2200.0000 mm^2"), "{info}");

    // Stored B-rep and regeneration from parameters both reproduce it.
    let verify = ok(&["part", "verify", s(&part)]);
    assert!(verify.contains("all identical"), "{verify}");
    assert!(!verify.contains("DIFFER"), "{verify}");

    // A clean close leaves no recovery journal behind.
    assert!(!dir.join("bracket.pxp.journal").exists());
}

#[test]
fn inch_template_takes_bare_numbers_in_inches() {
    let dir = scratch("in");
    let part = dir.join("plate.pxp");
    ok(&["part", "new", s(&part), "--units", "in"]);
    let created = ok(&["part", "block", s(&part), "--size", "1", "2", "3"]);
    let info = ok(&["part", "info", s(&part)]);
    assert!(info.contains("(inch part)"), "{info}");
    assert!(info.contains("volume 6.0000 in^3"), "{info}");
    assert!(info.contains(&format!("hash {}", created_hash(&created))));
    // The same block in a mm part (25.4, 50.8, 76.2 mm) has the same geometry.
    let mm_part = dir.join("plate-mm.pxp");
    ok(&["part", "new", s(&mm_part)]);
    let mm = ok(&[
        "part",
        "block",
        s(&mm_part),
        "--size",
        "25.4",
        "50.8",
        "76.2",
    ]);
    assert_eq!(created_hash(&mm), created_hash(&created));
}

#[test]
fn every_primitive_round_trips_and_errors_are_reported() {
    let dir = scratch("all");
    let part = dir.join("all.pxp");
    ok(&["part", "new", s(&part)]);
    ok(&[
        "part",
        "cylinder",
        s(&part),
        "--diameter",
        "6",
        "--height",
        "40",
        "--base",
        "5",
        "10",
        "-5",
    ]);
    ok(&[
        "part",
        "cone",
        s(&part),
        "--diameters",
        "20",
        "5",
        "--height",
        "15",
        "--axis",
        "0",
        "1",
        "1",
    ]);
    ok(&[
        "part",
        "sphere",
        s(&part),
        "--diameter",
        "14",
        "--center",
        "50",
        "0",
        "0",
    ]);
    ok(&[
        "part",
        "block",
        s(&part),
        "--size",
        "5",
        "5",
        "5",
        "--on-datum",
        "--name",
        "Cube",
    ]);
    let verify = ok(&["part", "verify", s(&part)]);
    assert!(
        verify.contains("verified") && verify.contains("4 solid(s)"),
        "{verify}"
    );

    // A zero-size block is refused and nothing is saved.
    let before = std::fs::read(&part).unwrap();
    let (success, _, stderr) = cli(&["part", "block", s(&part), "--size", "0", "1", "1"]);
    assert!(!success);
    assert!(stderr.contains("nothing saved"), "{stderr}");
    assert_eq!(std::fs::read(&part).unwrap(), before);

    // Usage errors exit with status 2.
    let out = Command::new(env!("CARGO_BIN_EXE_pxp-cli"))
        .args(["part", "block", s(&part)])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}
