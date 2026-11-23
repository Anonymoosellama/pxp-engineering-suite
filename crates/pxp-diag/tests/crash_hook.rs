// SPDX-License-Identifier: LGPL-2.1-or-later
//! The installed hook writes a report for a real panic. This is its own
//! test binary because a panic hook is process-wide.
#![allow(clippy::unwrap_used, clippy::panic, missing_docs)]

use pxp_diag::crash;

#[test]
fn a_panic_leaves_a_report_and_the_program_carries_on() {
    let dir = std::env::temp_dir().join(format!("pxp-crash-hook-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    crash::install(dir.clone(), "crash-hook-test");
    let worker = std::thread::Builder::new()
        .name("recompute-7".into())
        .spawn(|| panic!("index 9 out of range in the test"))
        .unwrap();
    assert!(worker.join().is_err(), "the thread panicked");
    let reports = crash::pending(&dir);
    assert_eq!(reports.len(), 1);
    let text = std::fs::read_to_string(&reports[0]).unwrap();
    assert!(text.contains("program: crash-hook-test"));
    assert!(text.contains("thread: recompute-7"));
    assert!(text.contains("panic: index 9 out of range in the test"));
    assert!(text.contains("crash_hook.rs"), "location recorded:\n{text}");
    assert_eq!(crash::clear(&dir).unwrap(), 1);
}
