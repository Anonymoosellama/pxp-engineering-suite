# ADR-0003: File container for .pxp

- **Status:** Proposed. SQLite is the working choice "unless the prototype
  finds a blocker" (master prompt); the prototype needs rusqlite from
  crates.io.
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

To be fixed by the prototype: `PRAGMA application_id` magic number, WAL
versus rollback journal for the recovery journal, page size, blob chunk size,
and the advisory-lock mechanism (an OS lock from `pxp-platform` in addition
to SQLite's own locking).

## Alternatives considered

- **ZIP of files (FreeCAD .FCStd):** rewritten whole on every save, no
  transactions. Rejected by the master prompt's reasoning.

## Consequences

- `pxp-format` depends on rusqlite (MIT) with the bundled SQLite (public
  domain): compatible with ADR-0001.
- The recovery journal can be a second SQLite file, written after every
  committed document transaction.
