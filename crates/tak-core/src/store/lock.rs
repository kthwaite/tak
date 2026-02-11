use fs2::FileExt;
use std::fs::{File, OpenOptions};
use std::path::Path;

use crate::error::{Result, TakError};

/// Acquire an exclusive lock on a file, returning the locked File handle.
/// Retries with exponential backoff (1ms to 512ms, ~1s total) before failing.
pub fn acquire_lock(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;

    let mut delay = std::time::Duration::from_millis(1);
    let max_delay = std::time::Duration::from_millis(512);

    loop {
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(file),
            Err(_) if delay <= max_delay => {
                std::thread::sleep(delay);
                delay *= 2;
            }
            Err(_) => {
                return Err(TakError::Locked(path.display().to_string()));
            }
        }
    }
}

/// Release lock explicitly (normally handled by Drop).
pub fn release_lock(file: File) -> Result<()> {
    file.unlock()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn acquire_and_release_lock() {
        let dir = tempdir().unwrap();
        let lock_path = dir.path().join("test.lock");

        let file = acquire_lock(&lock_path).unwrap();
        // Lock is held; trying to acquire again should fail
        assert!(acquire_lock(&lock_path).is_err());
        // Release
        release_lock(file).unwrap();
        // Can acquire again
        let _file = acquire_lock(&lock_path).unwrap();
    }

    #[test]
    fn acquire_fails_after_retries_exhausted() {
        let dir = tempdir().unwrap();
        let lock_path = dir.path().join("test-timeout.lock");

        let _file = acquire_lock(&lock_path).unwrap();

        let start = std::time::Instant::now();
        let result = acquire_lock(&lock_path);
        let elapsed = start.elapsed();

        assert!(result.is_err(), "should fail when lock is held");
        assert!(
            elapsed >= std::time::Duration::from_millis(500),
            "expected retry backoff, but elapsed was {elapsed:?}",
        );
    }
}
