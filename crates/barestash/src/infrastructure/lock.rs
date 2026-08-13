use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fs4::{FileExt, TryLockError};
use thiserror::Error;

const DEFAULT_RETRY_INTERVAL: Duration = Duration::from_millis(50);
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Error)]
pub enum FileLockError {
    #[error("failed to create the credential lock directory")]
    CreateDirectory(#[source] std::io::Error),
    #[error("failed to open the credential lock file")]
    Open(#[source] std::io::Error),
    #[error("failed to secure the credential lock file")]
    Permissions(#[source] std::io::Error),
    #[error("failed to acquire the credential lock")]
    Acquire(#[source] std::io::Error),
    #[error("Timed out waiting for the credential lock.")]
    Timeout,
    #[error("the credential lock task failed")]
    Join(#[from] tokio::task::JoinError),
}

/// A process-wide credential/config lock backed by the operating system.
///
/// The lock file intentionally remains on disk. The operating-system lock is
/// released automatically if a process exits, avoiding PID/staleness races.
#[derive(Clone, Debug)]
pub struct FileLock {
    path: PathBuf,
    retry_interval: Duration,
    timeout: Duration,
}

impl FileLock {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            retry_interval: DEFAULT_RETRY_INTERVAL,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    pub fn with_timing(
        path: impl Into<PathBuf>,
        retry_interval: Duration,
        timeout: Duration,
    ) -> Self {
        Self {
            path: path.into(),
            retry_interval,
            timeout,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Acquires the lock without blocking a Tokio worker thread.
    pub async fn acquire(&self) -> Result<FileLockGuard, FileLockError> {
        let lock = self.clone();
        tokio::task::spawn_blocking(move || lock.acquire_blocking()).await?
    }

    pub fn acquire_blocking(&self) -> Result<FileLockGuard, FileLockError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(FileLockError::CreateDirectory)?;
        }

        let file = open_lock_file(&self.path)?;
        let started_at = Instant::now();

        loop {
            match FileExt::try_lock(&file) {
                Ok(()) => return Ok(FileLockGuard { file }),
                Err(TryLockError::WouldBlock) => {
                    if started_at.elapsed() >= self.timeout {
                        return Err(FileLockError::Timeout);
                    }
                    std::thread::sleep(self.retry_interval);
                }
                Err(TryLockError::Error(error)) => {
                    return Err(FileLockError::Acquire(error));
                }
            }
        }
    }
}

#[derive(Debug)]
pub struct FileLockGuard {
    file: File,
}

impl Drop for FileLockGuard {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

fn open_lock_file(path: &Path) -> Result<File, FileLockError> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let file = options.open(path).map_err(FileLockError::Open)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(FileLockError::Permissions)?;
    }

    Ok(file)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};
    use std::thread;

    use super::*;

    #[test]
    fn serializes_independent_lock_instances() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("credentials.lock");
        let first = FileLock::with_timing(&path, Duration::from_millis(1), Duration::from_secs(1));
        let second = first.clone();
        let barrier = Arc::new(Barrier::new(2));
        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));

        let handles = [first, second].map(|lock| {
            let barrier = Arc::clone(&barrier);
            let active = Arc::clone(&active);
            let max_active = Arc::clone(&max_active);
            thread::spawn(move || {
                barrier.wait();
                let _guard = lock.acquire_blocking().expect("lock acquisition");
                let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                max_active.fetch_max(current, Ordering::SeqCst);
                thread::sleep(Duration::from_millis(10));
                active.fetch_sub(1, Ordering::SeqCst);
            })
        });

        for handle in handles {
            handle.join().expect("lock thread");
        }

        assert_eq!(max_active.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn times_out_while_another_handle_owns_the_lock() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("credentials.lock");
        let owner = FileLock::new(&path);
        let _guard = owner.acquire_blocking().expect("owner lock");
        let waiter =
            FileLock::with_timing(&path, Duration::from_millis(1), Duration::from_millis(5));

        assert!(matches!(
            waiter.acquire_blocking(),
            Err(FileLockError::Timeout)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn lock_file_is_user_only() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("credentials.lock");
        let lock = FileLock::new(&path);
        let _guard = lock.acquire_blocking().expect("lock acquisition");

        let mode = std::fs::metadata(path)
            .expect("lock metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}
