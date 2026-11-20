// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-compute: cancellation and deterministic parallelism
//!
//! ## Responsibility
//!
//! - [`CancelToken`]: the cooperative cancellation token every long loop
//!   checks, so the UI's cancel button always works (master prompt: "The UI
//!   thread never blocks... cooperative cancellation tokens checked inside
//!   every long loop").
//! - [`par_map`]: an order-preserving parallel map used by the recompute
//!   scheduler to run independent branches of the dependency graph on all
//!   cores.
//! - Later (Stage 0, sub-stage 0H and beyond): wgpu device management and the
//!   GPU/CPU dispatch rule (every GPU path has a CPU reference, an automatic
//!   fallback, and an agreement test).
//!
//! ## Current implementation and planned swap
//!
//! The master prompt specifies rayon for data-parallel work. This session has
//! no crates.io access, so [`par_map`] uses `std::thread::scope` with one
//! contiguous chunk per available core. The signature is rayon-shaped
//! (`Fn(&T) -> R + Sync`), so swapping the body for `par_iter().map()` later
//! changes no caller. Recorded in `docs/STATUS.md` under temporary
//! deviations.
//!
//! ## Data flow
//!
//! Foundation crate: used by `pxp-doc` (recompute) and later by CAM, FEA,
//! tessellation and slicing.
//!
//! ## Invariants
//!
//! - [`par_map`] returns results in input order regardless of thread timing,
//!   so parallel recompute is deterministic.
//! - A panic in a worker becomes [`ComputeError::WorkerPanicked`]; it never
//!   unwinds into the caller.
//!
//! ## Must never
//!
//! - Block the UI thread (callers run it on worker or document threads).
//! - Reorder results.
#![forbid(unsafe_code)]

use std::fmt;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Cooperative cancellation shared between the UI (which cancels) and
/// workers (which check). Cloning shares the same flag.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    /// A token that has not been cancelled.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests cancellation. Idempotent.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// True once [`CancelToken::cancel`] has been called on any clone.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    /// `Err(ComputeError::Cancelled)` if cancelled: use with `?` inside long
    /// loops.
    ///
    /// # Errors
    /// [`ComputeError::Cancelled`] after cancellation was requested.
    pub fn check(&self) -> Result<(), ComputeError> {
        if self.is_cancelled() {
            Err(ComputeError::Cancelled)
        } else {
            Ok(())
        }
    }
}

/// Errors from parallel execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComputeError {
    /// The operation observed a cancelled [`CancelToken`].
    Cancelled,
    /// A worker thread panicked; the message is the panic payload.
    WorkerPanicked(String),
}

impl fmt::Display for ComputeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => f.write_str("cancelled by the user"),
            Self::WorkerPanicked(m) => write!(f, "worker thread panicked: {m}"),
        }
    }
}

impl std::error::Error for ComputeError {}

/// Number of worker threads to use: the OS's available parallelism, or 1 if
/// it cannot be determined.
#[must_use]
pub fn worker_count() -> usize {
    std::thread::available_parallelism().map_or(1, NonZeroUsize::get)
}

/// Applies `f` to every item in parallel and returns the results **in input
/// order**.
///
/// Method: split `items` into at most [`worker_count`] contiguous chunks, run
/// each chunk on a scoped thread, then join the threads in chunk order and
/// concatenate. Ordering is therefore independent of scheduling. Small inputs
/// (fewer than 2 items, or a single core) run on the calling thread with no
/// spawn overhead. Complexity: O(n) calls of `f` plus O(workers) spawns.
///
/// # Errors
/// [`ComputeError::WorkerPanicked`] if `f` panicked on any item (the other
/// chunks still finish; their results are discarded).
pub fn par_map<T, R, F>(items: &[T], f: F) -> Result<Vec<R>, ComputeError>
where
    T: Sync,
    R: Send,
    F: Fn(&T) -> R + Sync,
{
    let workers = worker_count().min(items.len());
    if workers <= 1 {
        return std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            items.iter().map(&f).collect()
        }))
        .map_err(|p| ComputeError::WorkerPanicked(pxp_diag::panic_message(p.as_ref())));
    }
    let chunk_len = items.len().div_ceil(workers);
    std::thread::scope(|scope| {
        let handles: Vec<_> = items
            .chunks(chunk_len)
            .map(|chunk| scope.spawn(|| chunk.iter().map(&f).collect::<Vec<R>>()))
            .collect();
        let mut out = Vec::with_capacity(items.len());
        let mut failure = None;
        for handle in handles {
            match handle.join() {
                Ok(part) => out.extend(part),
                Err(p) => {
                    failure.get_or_insert_with(|| pxp_diag::panic_message(p.as_ref()));
                }
            }
        }
        failure.map_or(Ok(out), |m| Err(ComputeError::WorkerPanicked(m)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_order() {
        let input: Vec<u64> = (0..10_000).collect();
        let out = par_map(&input, |v| v * 2).unwrap();
        assert_eq!(out, input.iter().map(|v| v * 2).collect::<Vec<_>>());
    }

    #[test]
    fn empty_and_single() {
        assert!(par_map(&[] as &[u8], |v| *v).unwrap().is_empty());
        assert_eq!(par_map(&[5], |v| v + 1).unwrap(), vec![6]);
    }

    #[test]
    fn worker_panic_becomes_error() {
        let input: Vec<u32> = (0..100).collect();
        let r = par_map(&input, |v| {
            assert!(*v != 57, "boom at 57");
            *v
        });
        assert!(matches!(r, Err(ComputeError::WorkerPanicked(m)) if m.contains("boom at 57")));
    }

    #[test]
    fn cancel_token_is_shared() {
        let t = CancelToken::new();
        let t2 = t.clone();
        assert!(t.check().is_ok());
        t2.cancel();
        assert_eq!(t.check(), Err(ComputeError::Cancelled));
    }
}
