// SPDX-License-Identifier: LGPL-2.1-or-later
//! `pxp-cli mcp`: a Model Context Protocol server over stdio that runs the
//! same commands as the CLI, headless (master prompt, Stage 0).
//!
//! ## Transport and methods
//!
//! JSON-RPC 2.0, one message per line on stdin and stdout (MCP's stdio
//! transport). Nothing else is written to stdout; diagnostics go to stderr.
//!
//! | Method | Reply |
//! | --- | --- |
//! | `initialize` | protocol version (the client's if supported), `tools` capability, server info |
//! | `notifications/initialized` | none (a notification) |
//! | `ping` | `{}` |
//! | `tools/list` | every tool below with its JSON Schema |
//! | `tools/call` | `{content: [{type: "text", text}], isError}` |
//!
//! ## Tools
//!
//! Each tool turns its arguments into a CLI argument list and calls the
//! function `main` calls, so a tool and its CLI command cannot drift apart:
//! `pxp_part_block {file: "a.pxp", size: [10, 20, 30]}` runs exactly
//! `pxp-cli part block a.pxp --size 10 20 30`. A failed command is a tool
//! result with `isError: true` and the CLI's message, not a protocol error.
//!
//! With `--journal FILE.pxj` every tool call is also recorded as a journal
//! entry (`journal` module), so an MCP session replays like a CLI one.
//!
//! ## Must never
//!
//! - Write anything but protocol messages to stdout.
//! - Run a command a tool does not describe (arguments are mapped field by
//!   field; unknown fields are refused).

use crate::CliError;
use crate::journal::{self, PartHashes, Runner};
use serde_json::{Map, Value, json};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

/// MCP protocol revisions this server speaks, newest first.
const PROTOCOLS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

pub(crate) const USAGE: &str =
    "usage: pxp-cli mcp [--journal FILE.pxj]   (JSON-RPC 2.0 on stdin/stdout)";

/// One tool: name, description, the CLI command it runs, and its fields.
struct Tool {
    name: &'static str,
    description: &'static str,
    /// The fixed start of the CLI argument list (`["part", "block"]`).
    command: &'static [&'static str],
    fields: &'static [Field],
}

/// How a field becomes CLI arguments.
#[derive(Clone, Copy)]
enum Shape {
    /// A positional argument (string).
    Positional,
    /// `--flag VALUE` (number or formula string).
    Value(&'static str),
    /// `--flag V1 … Vn` (n numbers or formula strings).
    Values(&'static str, usize),
    /// `--flag` when true.
    Switch(&'static str),
    /// `--flag CHOICE` from a fixed list.
    Choice(&'static str, &'static [&'static str]),
}

struct Field {
    name: &'static str,
    description: &'static str,
    required: bool,
    shape: Shape,
}

const fn f(name: &'static str, description: &'static str, required: bool, shape: Shape) -> Field {
    Field {
        name,
        description,
        required,
        shape,
    }
}

const FILE: Field = f(
    "file",
    "Path of the .pxp part (relative to the server's directory).",
    true,
    Shape::Positional,
);
const NAME: Field = f("name", "Feature name.", false, Shape::Value("--name"));
const BASE: Field = f(
    "base",
    "Base point [x, y, z] in the part's units.",
    false,
    Shape::Values("--base", 3),
);
const AXIS: Field = f(
    "axis",
    "Axis direction [i, j, k].",
    false,
    Shape::Values("--axis", 3),
);

const TOOLS: &[Tool] = &[
    Tool {
        name: "pxp_part_new",
        description: "Create a new part from the mm or inch template (a datum CSYS at the origin).",
        command: &["part", "new"],
        fields: &[
            FILE,
            f(
                "units",
                "mm or in.",
                false,
                Shape::Choice("--units", &["mm", "in"]),
            ),
        ],
    },
    Tool {
        name: "pxp_part_block",
        description: "Add a block feature (sizes are numbers in the part's units or expression formulas) and save.",
        command: &["part", "block"],
        fields: &[
            FILE,
            f(
                "size",
                "[x, y, z] edge lengths.",
                true,
                Shape::Values("--size", 3),
            ),
            f(
                "on_datum",
                "Place the block on the datum CSYS.",
                false,
                Shape::Switch("--on-datum"),
            ),
            NAME,
        ],
    },
    Tool {
        name: "pxp_part_cylinder",
        description: "Add a cylinder feature and save.",
        command: &["part", "cylinder"],
        fields: &[
            FILE,
            f("diameter", "Diameter.", true, Shape::Value("--diameter")),
            f("height", "Height.", true, Shape::Value("--height")),
            BASE,
            AXIS,
            NAME,
        ],
    },
    Tool {
        name: "pxp_part_cone",
        description: "Add a cone feature (base and top diameters; top 0 for a pointed cone) and save.",
        command: &["part", "cone"],
        fields: &[
            FILE,
            f(
                "diameters",
                "[base, top] diameters.",
                true,
                Shape::Values("--diameters", 2),
            ),
            f("height", "Height.", true, Shape::Value("--height")),
            BASE,
            AXIS,
            NAME,
        ],
    },
    Tool {
        name: "pxp_part_sphere",
        description: "Add a sphere feature and save.",
        command: &["part", "sphere"],
        fields: &[
            FILE,
            f("diameter", "Diameter.", true, Shape::Value("--diameter")),
            f(
                "center",
                "Centre [x, y, z].",
                false,
                Shape::Values("--center", 3),
            ),
            NAME,
        ],
    },
    Tool {
        name: "pxp_part_info",
        description: "List a part's features with their states, kernels, geometry hashes, volumes and areas.",
        command: &["part", "info"],
        fields: &[FILE],
    },
    Tool {
        name: "pxp_part_verify",
        description: "Re-import every stored B-rep and regenerate every feature; fails unless hashes and bytes match.",
        command: &["part", "verify"],
        fields: &[FILE],
    },
    Tool {
        name: "pxp_eval",
        description: "Evaluate an NX-style expression with units through the document core.",
        command: &["eval"],
        fields: &[
            f(
                "formula",
                "The formula, e.g. \"7850[kg/m^3] * (100mm)^3\".",
                true,
                Shape::Positional,
            ),
            f(
                "units",
                "Display units: mm or in.",
                false,
                Shape::Choice("--units", &["mm", "in"]),
            ),
            f(
                "decimals",
                "Decimal places.",
                false,
                Shape::Value("--decimals"),
            ),
        ],
    },
    Tool {
        name: "pxp_commands",
        description: "List commands (optionally one application's) with status, ribbon placement and shortcut.",
        command: &["commands"],
        fields: &[f(
            "app",
            "Application id (modeling, drafting, …).",
            false,
            Shape::Positional,
        )],
    },
    Tool {
        name: "pxp_find",
        description: "Command Finder: search commands by label, id or description.",
        command: &["find"],
        fields: &[f("query", "Search text.", true, Shape::Positional)],
    },
    Tool {
        name: "pxp_apps",
        description: "List the six applications and the stage their commands arrive in.",
        command: &["apps"],
        fields: &[],
    },
    Tool {
        name: "pxp_status",
        description: "Which crates are implemented and which are planned.",
        command: &["status"],
        fields: &[],
    },
];

fn schema(tool: &Tool) -> Value {
    let mut props = Map::new();
    let mut required = Vec::new();
    for fld in tool.fields {
        let number_or_formula = json!({"oneOf": [{"type": "number"}, {"type": "string"}]});
        let mut p = match fld.shape {
            Shape::Positional => json!({"type": "string"}),
            Shape::Value(_) => number_or_formula,
            Shape::Values(_, n) => json!({"type": "array", "items": number_or_formula,
                                          "minItems": n, "maxItems": n}),
            Shape::Switch(_) => json!({"type": "boolean"}),
            Shape::Choice(_, options) => json!({"type": "string", "enum": options}),
        };
        if let Some(o) = p.as_object_mut() {
            o.insert("description".into(), json!(fld.description));
        }
        props.insert(fld.name.into(), p);
        if fld.required {
            required.push(fld.name);
        }
    }
    json!({"type": "object", "properties": props, "required": required,
           "additionalProperties": false})
}

fn scalar(v: &Value) -> Option<String> {
    match v {
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

/// The CLI arguments for a tool call, or why the arguments are wrong.
fn argv(tool: &Tool, args: &Map<String, Value>) -> Result<Vec<String>, String> {
    if let Some(unknown) = args
        .keys()
        .find(|k| !tool.fields.iter().any(|f| f.name == *k))
    {
        return Err(format!("{} has no argument '{unknown}'", tool.name));
    }
    let mut out: Vec<String> = tool.command.iter().map(|s| (*s).to_owned()).collect();
    for fld in tool.fields {
        let Some(v) = args.get(fld.name) else {
            if fld.required {
                return Err(format!("{} needs '{}'", tool.name, fld.name));
            }
            continue;
        };
        let bad = || format!("'{}' has the wrong type", fld.name);
        match fld.shape {
            Shape::Positional => out.push(v.as_str().ok_or_else(bad)?.to_owned()),
            Shape::Value(flag) => {
                out.push(flag.into());
                out.push(scalar(v).ok_or_else(bad)?);
            }
            Shape::Values(flag, n) => {
                let items = v.as_array().filter(|a| a.len() == n).ok_or_else(bad)?;
                out.push(flag.into());
                for item in items {
                    out.push(scalar(item).ok_or_else(bad)?);
                }
            }
            Shape::Switch(flag) => {
                if v.as_bool().ok_or_else(bad)? {
                    out.push(flag.into());
                }
            }
            Shape::Choice(flag, options) => {
                let s = v.as_str().filter(|s| options.contains(s)).ok_or_else(bad)?;
                out.push(flag.into());
                out.push(s.into());
            }
        }
    }
    Ok(out)
}

/// The server: what to run commands with, and where to record them.
pub(crate) struct Server {
    run: Runner,
    hashes: PartHashes,
    kernel: String,
    journal: Option<PathBuf>,
}

impl Server {
    pub(crate) fn new(
        run: Runner,
        hashes: PartHashes,
        kernel: &str,
        journal: Option<PathBuf>,
    ) -> Self {
        Self {
            run,
            hashes,
            kernel: kernel.to_owned(),
            journal,
        }
    }

    fn call_tool(&self, params: &Value) -> Value {
        let text_result = |text: String, error: bool| json!({"content": [{"type": "text", "text": text}], "isError": error});
        let name = params.get("name").and_then(Value::as_str).unwrap_or("");
        let Some(tool) = TOOLS.iter().find(|t| t.name == name) else {
            return text_result(format!("unknown tool '{name}'"), true);
        };
        let empty = Map::new();
        let args = params
            .get("arguments")
            .and_then(Value::as_object)
            .unwrap_or(&empty);
        let command = match argv(tool, args) {
            Ok(a) => a,
            Err(e) => return text_result(e, true),
        };
        let result = match &self.journal {
            Some(j) => journal::record_one(j, &command, self.run, self.hashes, &self.kernel),
            None => (self.run)(&command),
        };
        match result {
            Ok(out) if out.starts_with("error: ") => text_result(out, true),
            Ok(out) => text_result(out, false),
            Err(e) => text_result(e.to_string(), true),
        }
    }

    /// The reply to one message, or `None` for a notification.
    pub(crate) fn handle(&self, msg: &Value) -> Option<Value> {
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let result = match method {
            "initialize" => {
                let asked = params.get("protocolVersion").and_then(Value::as_str);
                let version = asked
                    .filter(|v| PROTOCOLS.contains(v))
                    .unwrap_or(PROTOCOLS[0]);
                Ok(json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": "pxp-cli", "version": env!("CARGO_PKG_VERSION")},
                    "instructions": "PXP Engineering Suite, headless. Tools run the pxp-cli commands of the same name; paths are relative to the server's directory.",
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": TOOLS.iter().map(|t| json!({
                "name": t.name, "description": t.description, "inputSchema": schema(t),
            })).collect::<Vec<_>>()})),
            "tools/call" => Ok(self.call_tool(&params)),
            _ if method.starts_with("notifications/") => return None,
            _ => Err(json!({"code": -32601, "message": format!("method not found: {method}")})),
        };
        // A message without an id is a notification: never answered.
        let id = id?;
        Some(match result {
            Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
            Err(e) => json!({"jsonrpc": "2.0", "id": id, "error": e}),
        })
    }

    /// Serves until stdin closes.
    pub(crate) fn serve(
        &self,
        input: impl BufRead,
        mut output: impl Write,
    ) -> Result<(), CliError> {
        for line in input.lines() {
            let line = line.map_err(|e| CliError::Command(e.to_string()))?;
            if line.trim().is_empty() {
                continue;
            }
            let reply = match serde_json::from_str::<Value>(&line) {
                Ok(msg) => self.handle(&msg),
                Err(e) => Some(json!({"jsonrpc": "2.0", "id": null,
                    "error": {"code": -32700, "message": format!("parse error: {e}")}})),
            };
            if let Some(r) = reply {
                writeln!(output, "{r}").map_err(|e| CliError::Command(e.to_string()))?;
                output
                    .flush()
                    .map_err(|e| CliError::Command(e.to_string()))?;
            }
        }
        Ok(())
    }
}

/// `pxp-cli mcp [--journal FILE]`.
pub(crate) fn run(
    args: &[String],
    runner: Runner,
    hashes: PartHashes,
    kernel: &str,
) -> Result<String, CliError> {
    let journal = match args {
        [] => None,
        [flag, file] if flag == "--journal" => Some(Path::new(file).to_path_buf()),
        [h] if matches!(h.as_str(), "help" | "--help" | "-h") => return Ok(format!("{USAGE}\n")),
        _ => return Err(CliError::Usage(USAGE.into())),
    };
    let server = Server::new(runner, hashes, kernel, journal);
    let stdin = std::io::stdin();
    server.serve(stdin.lock(), std::io::stdout().lock())?;
    Ok(String::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn echo(args: &[String]) -> Result<String, CliError> {
        if args.iter().any(|a| a == "boom") {
            return Err(CliError::Command("boom failed".into()));
        }
        Ok(args.join(" "))
    }

    // The `PartHashes` signature is fixed by the journal module.
    #[allow(clippy::unnecessary_wraps)]
    fn no_hashes(_: &Path) -> Result<Vec<(String, Option<u64>)>, CliError> {
        Ok(Vec::new())
    }

    fn server() -> Server {
        Server::new(echo, no_hashes, "test", None)
    }

    #[test]
    fn tool_arguments_become_the_cli_command() {
        let reply = server()
            .handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": {"name": "pxp_part_block",
                           "arguments": {"file": "a.pxp", "size": [10, "p0*2", 30.5], "on_datum": true}}}))
            .unwrap();
        assert_eq!(
            reply["result"]["content"][0]["text"],
            "part block a.pxp --size 10 p0*2 30.5 --on-datum"
        );
        assert_eq!(reply["result"]["isError"], false);
    }

    #[test]
    fn bad_arguments_and_failures_are_tool_errors() {
        let s = server();
        for args in [
            json!({"file": "a.pxp"}),
            json!({"file": "a.pxp", "size": [1, 2]}),
            json!({"file": "a.pxp", "size": [1, 2, 3], "colour": "red"}),
        ] {
            let r = s
                .handle(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
                    "params": {"name": "pxp_part_block", "arguments": args}}))
                .unwrap();
            assert_eq!(r["result"]["isError"], true, "{r}");
        }
        let r = s
            .handle(&json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
                "params": {"name": "pxp_find", "arguments": {"query": "boom"}}}))
            .unwrap();
        assert_eq!(r["result"]["isError"], true);
    }

    #[test]
    fn protocol_basics() {
        let s = server();
        let init = s
            .handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {"protocolVersion": "2025-03-26"}}))
            .unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert!(
            s.handle(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
                .is_none()
        );
        let list = s
            .handle(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}))
            .unwrap();
        assert_eq!(
            list["result"]["tools"].as_array().unwrap().len(),
            TOOLS.len()
        );
        let unknown = s
            .handle(&json!({"jsonrpc": "2.0", "id": 3, "method": "nope"}))
            .unwrap();
        assert_eq!(unknown["error"]["code"], -32601);
        let mut out = Vec::new();
        s.serve(&b"not json\n"[..], &mut out).unwrap();
        assert!(String::from_utf8(out).unwrap().contains("-32700"));
    }
}
