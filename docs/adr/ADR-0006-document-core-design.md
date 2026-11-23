# ADR-0006: Document core design and contract additions

- **Status:** Accepted (implemented in session 1, `crates/pxp-doc`,
  `crates/pxp-appkit`)
- **Date:** 2026-11-20; decision 1 revised 2026-11-21 (persistent maps)

## Decisions

1. **State representation.** A `DocState` is a persistent ordered map
   `imbl::OrdMap<ObjectId, Arc<Object>>` plus two derived indexes (reverse
   dependencies as `OrdMap<ObjectId, OrdSet<ObjectId>>`, expression names
   as `OrdMap<String, ObjectId>`). A transaction clones the state (O(1):
   the maps share structure) and mutates the clone (O(log n) per changed
   object, copying only the touched tree nodes). Commit swaps it in;
   failure or panic drops it. Rollback is therefore exact by construction,
   not by replaying inverse operations. Iteration order is the key order,
   exactly as with the `BTreeMap` it replaced, so saved files and hashes
   are unchanged.
2. **Undo entries are object deltas.** Each entry stores the `Arc` before and
   after of only the objects the transaction touched. Memory is proportional
   to the change, and the history is capped in estimated bytes (default
   256 MiB), always keeping the newest entry.
3. **Dependencies are stored on the dependant**, with the reverse index
   derived in exactly one function (`DocState::replace_raw`). Cycles are
   rejected when the edge is added. Deletion with dependants is refused
   unless the caller asks for `delete_with_dependants`.
4. **Out-of-date propagation is central.** At commit, every transitive
   dependant of a changed, deleted or (un)suppressed object is marked out of
   date by the core. An explicitly set state protects an object only from
   being marked because *its own* content changed, never from upstream
   changes (see the design note for the bug this prevents).
5. **Recompute is level-parallel and all-or-nothing** (master prompt). It
   commits every result, merged into the undo step of the triggering edit,
   or, on any failure, only the failure flags.
6. **Expressions are objects** of kind `core.expression`, evaluated eagerly
   on definition and re-evaluated by recompute.
7. **`Application` trait additions** beyond the master prompt:
   `name()`, `planned_stage()`, `update_policy(kind)` (default Manual) and
   `recompute(snapshot, object)` (default: a refusal that names the planned
   stage). The scheduler needs to know who regenerates each kind and when,
   and safe defaults mean an unimplemented kind is never auto-run.

## Alternatives rejected

- **Inverse-operation logs for rollback:** more code paths that must be
  exactly right, and a panic mid-operation can leave the log inconsistent.
- **Full-state snapshots for undo:** O(model) memory per step.
- **`BTreeMap` with whole-map clones** (session 1, when crates.io was not
  reachable): O(n) pointer copies per transaction and per recompute level.
- **`im` and `rpds`:** `im` is unmaintained (last release 2022; `imbl` is
  its maintained fork with the same API). `rpds` would work too; `imbl`
  was chosen for its ordered map with `Arc` sharing and a larger user base.
  Licence MPL-2.0: file-level copyleft, compatible with ADR-0001 and
  allowed in `deny.toml`.
- **A hash map (`imbl::HashMap`):** iteration order would no longer be
  deterministic by key, which the file writer and geometry hashes rely on.

## Known costs and follow-ups

- Resolved 2026-11-21: cloning `DocState` was O(n) pointer copies per
  transaction (about 1 ms per 100 000 objects). With `imbl` it is O(1);
  the benchmark `doc/edit-one-of-100k` (`crates/pxp-doc/benches/doc.rs`)
  holds it to the budget in `docs/design/performance-budgets.md`. The
  public API did not change.
- Parallel recompute uses `std::thread::scope`; swap to rayon in
  `pxp-compute` when crates are available.
- Persistent naming (building stable names from `TopoHistory`) is not
  implemented yet; it is the first Stage 0 item after the kernel bake-off.
