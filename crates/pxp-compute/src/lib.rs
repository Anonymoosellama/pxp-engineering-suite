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
//! ## Implementation
//!
//! [`par_map`] runs on rayon's global work-stealing pool (the master prompt's
//! choice for data-parallel work). Session 1 used `std::thread::scope` while
//! crates.io was unreachable; the signature was rayon-shaped from the start,
//! so the swap changed no caller.
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

/// Number of worker threads in the parallel pool.
#[must_use]
pub fn worker_count() -> usize {
    rayon::current_num_threads()
}

/// Applies `f` to every item in parallel and returns the results **in input
/// order**.
///
/// Method: rayon's indexed parallel iterator over the slice, collected into a
/// `Vec`, which places each result at its input index regardless of which
/// thread computed it or when. Work stealing balances uneven item costs (one
/// slow feature does not hold up a whole chunk). O(n) calls of `f`.
///
/// # Errors
/// [`ComputeError::WorkerPanicked`] if `f` panicked on any item. Rayon
/// re-raises a worker's panic in the calling thread; it is caught here so it
/// never unwinds into the caller.
pub fn par_map<T, R, F>(items: &[T], f: F) -> Result<Vec<R>, ComputeError>
where
    T: Sync,
    R: Send,
    F: Fn(&T) -> R + Sync,
{
    use rayon::prelude::*;
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        items.par_iter().map(&f).collect()
    }))
    .map_err(|p| ComputeError::WorkerPanicked(pxp_diag::panic_message(p.as_ref())))
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
