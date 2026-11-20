# ADR-0006: Document core design and contract additions

- **Status:** Accepted (implemented in session 1, `crates/pxp-doc`,
  `crates/pxp-appkit`)
- **Date:** 2026-11-20

## Decisions

1. **State representation.** A `DocState` is a `BTreeMap<ObjectId,
   Arc<Object>>` plus two derived indexes (reverse dependencies, expression
   names). A transaction clones the state (pointer copies only) and mutates
   the clone. Commit swaps it in; failure or panic drops it. Rollback is
   therefore exact by construction, not by replaying inverse operations.
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
- **A persistent hash map now:** no crates.io access. `im` or `rpds`
  (MPL-2.0, compatible with ADR-0001) are the planned replacement.

## Known costs and follow-ups

- Cloning `DocState` is O(n) pointer copies per transaction and per recompute
  level: about 1 ms per 100 000 objects. Replace the map with a persistent
  one when crates are available, measured against the Stage 0 performance
  budgets (0J). The public API does not change.
- Parallel recompute uses `std::thread::scope`; swap to rayon in
  `pxp-compute` when crates are available.
- Persistent naming (building stable names from `TopoHistory`) is not
  implemented yet; it is the first Stage 0 item after the kernel bake-off.
