# ADR-0005: Plugin interface

- **Status:** Proposed (direction fixed by the master prompt; the WIT world
  is not written yet).
- **Date:** 2026-11-20

## Decision

Third-party extensions are WebAssembly components run in wasmtime against a
versioned WIT world (`pxp:plugin@0.1.0`). Built-in applications stay
statically linked crates. Native Rust dylib plugins are not used: Rust has
no stable ABI.

A plugin may contribute commands, feature types, object kinds with their
schema, importers and exporters, CAM operations and post-processor layers.
SolidCraft's `craft:plugin` interface is prior art.

## Consequences found in session 1

- **Licence:** wasmtime is `Apache-2.0 WITH LLVM-exception`; the exception's
  GPLv2-compatibility clause makes it compatible with LGPL-2.1-or-later
  (ADR-0001) without moving binaries to LGPL-3.0.
- **Runtime strings:** plugin commands arrive at run time, so
  `pxp_appkit::CommandSpec` text fields are `Cow<'static, str>`, and each
  application's "switch to" command ID derives from its application ID
  (`app.switch.<app-id>`). A registry test registers two non-built-in
  applications to keep this working.
- **Unknown kinds:** objects of a kind whose plugin is not installed are kept
  byte for byte and get the manual update policy (never regenerated, never
  dropped). Implemented and tested in `pxp-doc` and `pxp-appkit`.
- **Sandbox:** plugins and posts get no file or network access beyond
  host-provided streams.
