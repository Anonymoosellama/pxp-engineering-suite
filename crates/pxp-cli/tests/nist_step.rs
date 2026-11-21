// SPDX-License-Identifier: LGPL-2.1-or-later
//! One NIST MBE PMI test model (FTC-11, AP203, public domain; see
//! `tests/data/NIST-NOTICE.md`) through `pxp-cli stepstress` on every CI
//! target, so STEP import, the round trips and the boolean identities run
//! on real geometry with every push. The full 39-file NIST corpus runs in
//! the nightly workflow (`tools/fetch_nist_step.py`).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use std::process::Command;

#[test]
fn nist_ftc_11_passes_every_check() {
    let file = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/nist_ftc_11_asme1_rb.stp"
    );
    let out = Command::new(env!("CARGO_BIN_EXE_pxp-cli"))
        .args([
            "stepstress",
            "--repeat",
            "2",
            "--format",
            "tsv",
            "--header",
            "--quiet",
            file,
        ])
        .output()
        .expect("run pxp-cli");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mut lines = stdout.lines();
    let header: Vec<&str> = lines.next().unwrap().split('\t').collect();
    let row: Vec<&str> = lines.next().unwrap().split('\t').collect();
    let col = |name: &str| row[header.iter().position(|h| *h == name).unwrap()];
    assert_eq!(col("status"), "pass", "{stdout}");
    assert_eq!(col("bodies"), "1");
    assert_eq!(col("merged_faces"), "6");
    // The file states no validation properties; every other check applies.
    assert_eq!(col("reference"), "n/a");
    for check in [
        "import",
        "native-roundtrip",
        "step-roundtrip",
        "box-identity",
        "half-split",
        "drill",
    ] {
        assert_eq!(col(check), "pass", "{check}: {stdout}");
    }
    assert_eq!(col("deterministic"), "yes");
}
