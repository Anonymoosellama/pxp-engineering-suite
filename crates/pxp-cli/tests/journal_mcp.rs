// SPDX-License-Identifier: LGPL-2.1-or-later
//! Journaling and the MCP server, end to end through the real binary:
//!
//! - every journal under `tests/journals/` replays with identical geometry
//!   hashes and outcomes (journals are regression tests);
//! - a journal with one hash changed fails its replay and names the feature;
//! - an MCP session (initialize, tools/list, tool calls, a refused call)
//!   recorded with `--journal` replays like a CLI session.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_pxp-cli")
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

fn replay(journal: &Path) -> (bool, String) {
    let out = Command::new(bin())
        .args(["journal", "replay"])
        .arg(journal)
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

#[test]
fn committed_journals_replay_identically() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/journals");
    let mut n = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "pxj") {
            let (ok, text) = replay(&path);
            assert!(ok, "{} did not replay:\n{text}", path.display());
            assert!(text.contains(" 0 difference(s)"), "{text}");
            n += 1;
        }
    }
    assert!(n > 0, "no journals in {}", dir.display());
}

#[test]
fn a_changed_hash_fails_the_replay() {
    let src =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/journals/primitives-mm-and-inch.pxj");
    let text = std::fs::read_to_string(&src).unwrap();
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    // Change the block's hash in its own entry (line 3: the first block).
    let mut entry: Value = serde_json::from_str(&lines[2]).unwrap();
    entry["parts"]["bracket.pxp"]["Base"] = json!("0000000000000000");
    lines[2] = entry.to_string();
    let dir = scratch("tampered");
    let tampered = dir.join("tampered.pxj");
    std::fs::write(&tampered, lines.join("\n") + "\n").unwrap();
    let (ok, out) = replay(&tampered);
    assert!(!ok, "a changed hash must fail:\n{out}");
    assert!(
        out.contains("DIFF") && out.contains("Base hash 0000000000000000 recorded"),
        "{out}"
    );
}

#[test]
fn an_mcp_session_is_journaled_and_replays() {
    let dir = scratch("mcp");
    let mut child = Command::new(bin())
        .args(["mcp", "--journal", "session.pxj"])
        .current_dir(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut call = |id: u64, method: &str, params: Value| -> Value {
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(stdin, "{msg}").unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        out.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    };
    let init = call(
        1,
        "initialize",
        json!({"protocolVersion": "2025-06-18", "capabilities": {}}),
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "pxp-cli");
    let tools = call(2, "tools/list", json!({}));
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"pxp_part_block") && names.contains(&"pxp_part_verify"));
    let tool =
        |id, name: &str, arguments: Value| (id, json!({"name": name, "arguments": arguments}));
    for (id, params) in [
        tool(3, "pxp_part_new", json!({"file": "m.pxp", "units": "mm"})),
        tool(
            4,
            "pxp_part_block",
            json!({"file": "m.pxp", "size": [10, 20, "p1 / 2"]}),
        ),
        tool(
            5,
            "pxp_part_sphere",
            json!({"file": "m.pxp", "diameter": 7, "center": [5, 5, 30]}),
        ),
    ] {
        let r = call(id, "tools/call", params);
        assert_eq!(r["result"]["isError"], false, "{r}");
    }
    let refused = call(
        6,
        "tools/call",
        json!({"name": "pxp_part_block",
        "arguments": {"file": "m.pxp", "size": [-1, 1, 1]}}),
    );
    assert_eq!(refused["result"]["isError"], true, "{refused}");
    let info = call(
        7,
        "tools/call",
        json!({"name": "pxp_part_info", "arguments": {"file": "m.pxp"}}),
    );
    let text = info["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("Block") && text.contains("hash"), "{text}");
    drop(stdin);
    assert!(child.wait().unwrap().success());
    let (ok, replayed) = replay(&dir.join("session.pxj"));
    assert!(ok, "{replayed}");
    assert!(
        replayed.contains("5 commands") && replayed.contains(" 0 difference(s)"),
        "{replayed}"
    );
}
