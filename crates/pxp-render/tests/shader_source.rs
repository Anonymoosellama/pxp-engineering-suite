// SPDX-License-Identifier: LGPL-2.1-or-later
//! Source checks on `shaders.wgsl` that hold on every OS, so a shader that
//! only one backend's compiler rejects fails on Linux CI too, not just on
//! the Windows runners.
//!
//! Why this exists: wgpu turns WGSL into HLSL through naga on Windows, and
//! naga copies a trailing `discard;` into HLSL as the function's last
//! statement. The Direct3D compiler (FXC) then refuses the whole shader
//! with X3507 "not all control paths return a value", so pipeline creation
//! panics and no viewport can be drawn. Vulkan, Metal and GL accept the
//! same WGSL, so the GPU tests pass everywhere except Windows. The rule
//! enforced here is the one the shader file follows: a function with a
//! return type ends with `return`; `discard` only appears inside an `if`.
#![allow(clippy::expect_used, missing_docs)]

/// The shader source exactly as `gpu.rs` compiles it.
const SHADERS: &str = include_str!("../src/shaders.wgsl");

/// One top-level function: its header line and its last statement.
struct Function<'a> {
    header: &'a str,
    returns_value: bool,
    last_statement: &'a str,
}

/// Split the file into top-level functions.
///
/// This leans on the file's formatting, which `shaders.wgsl` keeps: a
/// top-level function starts with `fn ` at column 0 (attributes such as
/// `@fragment` sit on the line above) and ends with a `}` alone at column
/// 0. Comment lines and blank lines are not statements, so they are
/// skipped when finding the last statement.
fn functions(source: &str) -> Vec<Function<'_>> {
    let mut found = Vec::new();
    let mut current: Option<(&str, &str)> = None; // (header, last statement)
    for line in source.lines() {
        if line.starts_with("fn ") {
            current = Some((line, ""));
            continue;
        }
        let Some((header, last)) = current.as_mut() else {
            continue;
        };
        if line == "}" {
            found.push(Function {
                header,
                // `-> type` between the parameter list and the body means
                // the function must return a value on every path.
                returns_value: header.contains("->"),
                last_statement: last,
            });
            current = None;
            continue;
        }
        let trimmed = line.trim();
        if !trimmed.is_empty() && !trimmed.starts_with("//") {
            *last = trimmed;
        }
    }
    found
}

#[test]
fn every_function_with_a_return_type_ends_in_return() {
    let all = functions(SHADERS);
    // Guard the parser itself: if the formatting assumption breaks, the
    // check below would pass vacuously.
    assert!(
        all.iter().any(|f| f.header.starts_with("fn sil_fs")),
        "no top-level functions found; has the shader file's layout changed?"
    );
    let offenders: Vec<String> = all
        .iter()
        .filter(|f| f.returns_value && !f.last_statement.starts_with("return"))
        .map(|f| format!("{}  (last statement: `{}`)", f.header, f.last_statement))
        .collect();
    assert!(
        offenders.is_empty(),
        "these shader functions do not end in `return`, which the Direct3D \
         compiler rejects (X3507) on Windows; put `discard` inside an `if` \
         and return last:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn the_check_catches_a_trailing_discard() {
    // The shape that broke Windows CI at b4d62b7.
    let bad = "fn f() -> @location(0) vec4<f32> {\n    if (x) {\n        return c;\n    }\n    discard;\n}\n";
    let found = functions(bad);
    let f = found.first().expect("one function");
    assert!(f.returns_value);
    assert_eq!(f.last_statement, "discard;");
}
