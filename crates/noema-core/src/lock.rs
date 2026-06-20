use fs4::FileExt;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::error::Result;

pub struct FileLock {
    file: File,
    path: PathBuf,
}

impl FileLock {
    pub fn exclusive(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&path)?;
        file.lock_exclusive()?;
        Ok(Self { file, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    atomic_write_locked(&path.with_extension("lock"), path, bytes)
}

pub fn atomic_write_locked(lock_path: &Path, path: &Path, bytes: &[u8]) -> Result<()> {
    let _lock = FileLock::exclusive(lock_path)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("tmp.{}.{}", std::process::id(), Uuid::new_v4()));
    {
        let mut file = File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    std::fs::rename(tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        atomic_write(&path, b"one").unwrap();
        atomic_write(&path, b"two").unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), "two");
    }

    #[test]
    fn lock_file_is_created() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tenant.lock");
        let lock = FileLock::exclusive(&path).unwrap();
        assert_eq!(lock.path(), path);
    }
}
