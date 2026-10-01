//! Durable wrapper over Tantivy directories that retries transient OS file locks.
//!
//! On Windows, Tantivy file operations can encounter transient `PermissionDenied`
//! or sharing violations (OS error 5, 32, 33) due to antivirus scanning, background
//! indexing, or delayed file descriptor cleanup. `RetryDirectory` wraps any underlying
//! directory and retries mutations with exponential backoff.

use std::fmt::Debug;
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::thread::sleep;
use std::time::Duration;

use tantivy::directory::error::{DeleteError, LockError, OpenReadError, OpenWriteError};
use tantivy::directory::{Directory, DirectoryLock, FileHandle, Lock, WatchCallback, WatchHandle, WritePtr};

/// Exponential retry configuration for transient file system errors.
const WRITE_MAX_RETRIES: usize = 12;
const WRITE_INITIAL_BACKOFF_MS: u64 = 5;
const WRITE_MAX_BACKOFF_MS: u64 = 250;

const DELETE_MAX_RETRIES: usize = 3;
const DELETE_INITIAL_BACKOFF_MS: u64 = 10;

/// Returns true if an IO error corresponds to transient file lock/sharing violations.
fn is_transient_fs_error(err: &io::Error) -> bool {
    if err.kind() == io::ErrorKind::PermissionDenied {
        return true;
    }
    #[cfg(windows)]
    {
        if let Some(raw_os_error) = err.raw_os_error() {
            // ERROR_ACCESS_DENIED (5), ERROR_SHARING_VIOLATION (32), ERROR_LOCK_VIOLATION (33)
            return raw_os_error == 5 || raw_os_error == 32 || raw_os_error == 33;
        }
    }
    false
}

/// Directory wrapper retrying transient filesystem lock errors.
#[derive(Debug, Clone)]
pub struct RetryDirectory<D: Directory + Clone> {
    inner: D,
}

impl<D: Directory + Clone> RetryDirectory<D> {
    /// Wrap an existing directory with retry logic.
    pub fn new(inner: D) -> Self {
        Self { inner }
    }

    /// Access the underlying directory.
    pub fn inner(&self) -> &D {
        &self.inner
    }
}

impl<D: Directory + Clone> Directory for RetryDirectory<D> {
    fn get_file_handle(&self, path: &Path) -> Result<Arc<dyn FileHandle>, OpenReadError> {
        self.inner.get_file_handle(path)
    }

    fn delete(&self, path: &Path) -> Result<(), DeleteError> {
        let mut delay = Duration::from_millis(DELETE_INITIAL_BACKOFF_MS);
        for attempt in 0..DELETE_MAX_RETRIES {
            match self.inner.delete(path) {
                Ok(()) => return Ok(()),
                Err(DeleteError::IoError { io_error, filepath }) => {
                    if is_transient_fs_error(&io_error) && attempt + 1 < DELETE_MAX_RETRIES {
                        tracing::warn!(
                            attempt = attempt + 1,
                            path = %path.display(),
                            "Transient lock during Tantivy delete, retrying..."
                        );
                        sleep(delay);
                        delay = delay.saturating_mul(2);
                    } else {
                        return Err(DeleteError::IoError { io_error, filepath });
                    }
                }
                Err(e) => return Err(e),
            }
        }
        self.inner.delete(path)
    }

    fn exists(&self, path: &Path) -> Result<bool, OpenReadError> {
        self.inner.exists(path)
    }

    fn open_write(&self, path: &Path) -> Result<WritePtr, OpenWriteError> {
        let mut delay = Duration::from_millis(WRITE_INITIAL_BACKOFF_MS);
        for attempt in 0..WRITE_MAX_RETRIES {
            match self.inner.open_write(path) {
                Ok(ptr) => return Ok(ptr),
                Err(OpenWriteError::IoError { io_error, filepath }) => {
                    if is_transient_fs_error(&io_error) && attempt + 1 < WRITE_MAX_RETRIES {
                        tracing::warn!(
                            attempt = attempt + 1,
                            path = %path.display(),
                            "Transient lock during Tantivy open_write, retrying..."
                        );
                        sleep(delay);
                        delay = (delay.saturating_mul(2)).min(Duration::from_millis(WRITE_MAX_BACKOFF_MS));
                    } else {
                        return Err(OpenWriteError::IoError { io_error, filepath });
                    }
                }
                Err(e) => return Err(e),
            }
        }
        self.inner.open_write(path)
    }

    fn atomic_read(&self, path: &Path) -> Result<Vec<u8>, OpenReadError> {
        self.inner.atomic_read(path)
    }

    fn atomic_write(&self, path: &Path, data: &[u8]) -> Result<(), io::Error> {
        let mut delay = Duration::from_millis(WRITE_INITIAL_BACKOFF_MS);
        for attempt in 0..WRITE_MAX_RETRIES {
            match self.inner.atomic_write(path, data) {
                Ok(()) => return Ok(()),
                Err(io_error) => {
                    if is_transient_fs_error(&io_error) && attempt + 1 < WRITE_MAX_RETRIES {
                        tracing::warn!(
                            attempt = attempt + 1,
                            path = %path.display(),
                            "Transient lock during Tantivy atomic_write, retrying..."
                        );
                        sleep(delay);
                        delay = (delay.saturating_mul(2)).min(Duration::from_millis(WRITE_MAX_BACKOFF_MS));
                    } else {
                        return Err(io_error);
                    }
                }
            }
        }
        self.inner.atomic_write(path, data)
    }

    fn sync_directory(&self) -> Result<(), io::Error> {
        self.inner.sync_directory()
    }

    fn watch(&self, watch_callback: WatchCallback) -> Result<WatchHandle, tantivy::TantivyError> {
        self.inner.watch(watch_callback)
    }

    fn acquire_lock(&self, lock: &Lock) -> Result<DirectoryLock, LockError> {
        self.inner.acquire_lock(lock)
    }
}
