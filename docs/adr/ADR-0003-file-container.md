# ADR-0003: File container for .pxp

- **Status:** Proposed. SQLite is the working choice "unless the prototype
  finds a blocker" (master prompt). Session 2 implemented it (`pxp-format`)
  and found no blocker; acceptance is decided at the Stage 0 gate.
- **Date:** 2026-11-20

## Context

One `.pxp` file holds every application's data for a part. The file must
survive power loss mid-save, load large sections lazily, save incrementally,
and be inspectable with standard tools.

## Decision (working)

A single SQLite database per part, following the master prompt's logical
schema (`meta`, `objects`, `deps`, `topo_names`, `expressions`, `geometry`,
`components`, `blobs`), with these refinements from the session 1 document
core:

- **Expressions are objects.** The document core stores expressions as
  objects of kind `core.expression` (name, formula, cached value), so undo,
  dependencies and change notification treat them like everything else. The
  `expressions` table becomes a view over `objects` (or is dropped), keeping
  one source of truth.
- **Dependencies live on the object.** `pxp-doc` keeps each object's
  dependencies in the object itself and derives the reverse index. The
  `deps` table is written from them on save and is the on-disk index;
  readers rebuild the in-memory reverse index on load.
- **State and diagnostic are columns.** `objects.state` plus a nullable
  `objects.diagnostic TEXT`, because a failure reason must survive save and
  reload ("never dropped silently").
- **Opaque payloads round-trip.** `payload` stays an opaque, versioned blob
  per kind; unknown kinds are written back byte for byte (tested in
  `pxp-doc`: `unknown_kind_payload_is_preserved_byte_for_byte`).

Fixed by the session 2 implementation (`pxp-format`, design note
`docs/design/file-format.md`):

- `PRAGMA application_id = 0x50585031` ("PXP1"), `user_version` = schema
  version (1).
- Whole-file saves to a temporary file, then an fsync-and-rename replace
  (`pxp_platform::replace_file`); three rolling backups. Incremental saves
  are a later optimisation: at Stage 0 sizes a full rewrite takes
  milliseconds and is simplest to prove crash-safe.
- Recovery journal: a separate SQLite file (`.journal`, rollback journal,
  `synchronous=FULL`) holding each committed transaction's changed objects,
  tied to the saved file by a random save token.
- Payload chunk size 8 MiB; the default SQLite page size.
- Lock: an OS advisory lock on a `.lock` sidecar (`std::fs::File::try_lock`)
  in addition to SQLite's own locking.
- Geometry: the master prompt's `geometry` table is not needed yet. A
  feature's exact B-rep is its payload, in the stored-shape envelope
  (backend id + the backend's native bytes); its `geometry_hash` is a
  property. `topo_names` and `components` arrive with persistent naming and
  assemblies (Stage 1).

## Alternatives considered

- **ZIP of files (FreeCAD .FCStd):** rewritten whole on every save, no
  transactions. Rejected by the master prompt's reasoning.

## Consequences

- `pxp-format` depends on rusqlite (MIT) with the bundled SQLite (public
  domain): compatible with ADR-0001.
- The recovery journal can be a second SQLite file, written after every
  committed document transaction.
