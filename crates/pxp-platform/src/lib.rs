// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-platform: OS abstraction
//!
//! The only crate allowed to contain operating-system-specific code
//! (master prompt). Everything else asks this crate.
//!
//! ## Responsibility
//!
//! - [`FileLock`]: an advisory, exclusive lock on a sidecar file, so two
//!   sessions never write the same part. Backed by the OS (`flock` on Unix,
//!   `LockFileEx` on Windows) through `std::fs::File::try_lock`, so the lock
//!   is released automatically if the process dies.
//! - [`replace_file`]: durable, atomic replacement of a file by a fully
//!   written temporary file (fsync the data, rename over the target, fsync the
//!   directory on Unix).
//! - Later in Stage 0: locations of layered customer defaults, the per-user
//!   data and crash-report directories, high-DPI and IME hooks.
//!
//! ## Data flow
//!
//! Called by `pxp-format` (locks, saves) and later `pxp-diag` and `pxp-ui`.
//!
//! ## Invariants
//!
//! - After [`replace_file`] returns `Ok`, the target holds the complete new
//!   content even across a power cut; if it returns `Err` or the machine
//!   crashes during the call, the target holds either the old or the new
//!   content, never a mixture.
//! - A [`FileLock`] is held until it is dropped.
//!
//! ## Must never
//!
//! - Contain domain logic.
//! - Panic because a path, permission or environment variable is missing.
#![forbid(unsafe_code)]

use std::fmt;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Stage of the master prompt in which this crate's remaining planned
/// functionality (defaults locations, DPI, IME) is scheduled. Shown by
/// `pxp-cli status`.
pub const PLANNED_STAGE: u8 = 0;

/// Errors from platform operations.
#[derive(Debug)]
pub enum PlatformError {
    /// Another process holds the lock. `holder` is the text the holder wrote
    /// into the lock file (process ID and host), if readable.
    Locked {
        /// The lock file.
        path: PathBuf,
        /// What the holder wrote, if anything.
        holder: String,
    },
    /// An I/O error, with the path involved.
    Io {
        /// What was being done.
        action: &'static str,
        /// The path involved.
        path: PathBuf,
        /// The underlying error.
        source: io::Error,
    },
}

impl fmt::Display for PlatformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Locked { path, holder } => {
                write!(
                    f,
                    "{} is in use by another session ({holder})",
                    path.display()
                )
            }
            Self::Io {
                action,
                path,
                source,
            } => {
                write!(f, "could not {action} {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for PlatformError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Locked { .. } => None,
        }
    }
}

fn io_err<'a>(
    action: &'static str,
    path: &'a Path,
) -> impl FnOnce(io::Error) -> PlatformError + 'a {
    move |source| PlatformError::Io {
        action,
        path: path.to_path_buf(),
        source,
    }
}

/// An exclusive advisory lock on a sidecar lock file.
///
/// Why a sidecar: parts are saved by writing a new file and renaming it over
/// the old one, which would orphan a lock held on the part file itself. The
/// lock file (`part.pxp.lock`) stays put across saves.
///
/// The lock file is left on disk when the lock is released; a later
/// [`FileLock::try_acquire`] succeeds because the OS lock, not the file's
/// existence, is what counts. A crashed session therefore never leaves a
/// part locked.
#[derive(Debug)]
pub struct FileLock {
    file: File,
    path: PathBuf,
}

impl FileLock {
    /// Tries to take the lock without waiting.
    ///
    /// # Errors
    /// [`PlatformError::Locked`] if another process holds it;
    /// [`PlatformError::Io`] if the lock file cannot be created or locked.
    pub fn try_acquire(path: &Path) -> Result<Self, PlatformError> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(io_err("open lock file", path))?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                let holder = std::fs::read_to_string(path)
                    .unwrap_or_default()
                    .trim()
                    .to_owned();
                return Err(PlatformError::Locked {
                    path: path.to_path_buf(),
                    holder,
                });
            }
            Err(TryLockError::Error(e)) => return Err(io_err("lock", path)(e)),
        }
        // Record who holds it (informational only; the OS lock is the truth).
        let note = format!("pid {} on {}\n", std::process::id(), host_name());
        file.set_len(0).map_err(io_err("write lock file", path))?;
        file.write_all(note.as_bytes())
            .map_err(io_err("write lock file", path))?;
        Ok(Self {
            file,
            path: path.to_path_buf(),
        })
    }

    /// The lock file's path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        // Unlocking can only fail if the handle is already invalid; closing
        // the handle releases the lock in any case.
        let _ = self.file.unlock();
    }
}

fn host_name() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "this computer".to_owned())
}

/// Durably replaces `target` with the already written file `temp`.
///
/// Steps: fsync `temp`'s data, rename it over `target` (atomic on every
/// supported OS; on Windows `std::fs::rename` replaces an existing file), then
/// on Unix fsync the containing directory so the rename itself survives a
/// power cut. `temp` must be on the same file system as `target` (put it in
/// the same directory).
///
/// # Errors
/// [`PlatformError::Io`] naming the failing step.
pub fn replace_file(temp: &Path, target: &Path) -> Result<(), PlatformError> {
    // Write access: Windows' FlushFileBuffers refuses read-only handles
    // ("Access is denied"); Unix accepts either.
    OpenOptions::new()
        .write(true)
        .open(temp)
        .and_then(|f| f.sync_all())
        .map_err(io_err("flush", temp))?;
    std::fs::rename(temp, target).map_err(io_err("replace", target))?;
    // Windows offers no directory-handle fsync; NTFS journals the rename.
    #[cfg(unix)]
    sync_parent_dir(target)?;
    Ok(())
}

#[cfg(unix)]
fn sync_parent_dir(path: &Path) -> Result<(), PlatformError> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    File::open(dir)
        .and_then(|d| d.sync_all())
        .map_err(io_err("flush directory", dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pxp-platform-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn second_lock_is_refused_until_first_drops() {
        let dir = temp_dir("lock");
        let path = dir.join("part.pxp.lock");
        let first = FileLock::try_acquire(&path).unwrap();
        match FileLock::try_acquire(&path) {
            Err(PlatformError::Locked { holder, .. }) => assert!(holder.contains("pid")),
            other => panic!("expected Locked, got {other:?}"),
        }
        drop(first);
        assert!(FileLock::try_acquire(&path).is_ok(), "released on drop");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn replace_file_swaps_content() {
        let dir = temp_dir("replace");
        let (target, temp) = (dir.join("a.pxp"), dir.join("a.pxp.tmp"));
        std::fs::write(&target, b"old").unwrap();
        std::fs::write(&temp, b"new").unwrap();
        replace_file(&temp, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert!(!temp.exists());
        assert!(matches!(
            replace_file(&temp, &target),
            Err(PlatformError::Io { .. })
        ));
        let _ = std::fs::remove_dir_all(dir);
    }
}
