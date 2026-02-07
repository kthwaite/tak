use fs2::FileExt;
use std::fs::{File, OpenOptions};
use std::path::Path;

use crate::error::{Result, TakError};

/// Acquire an exclusive lock on a file, returning the locked File handle.
/// The lock is released when the File is dropped.
pub fn acquire_lock(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;

    file.try_lock_exclusive().map_err(|_| {
        TakError::Locked(path.display().to_string())
    })?;

    Ok(file)
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
}
