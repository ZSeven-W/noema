use async_trait::async_trait;

use crate::error::{NoemaError, Result};

#[async_trait]
pub trait ObjectStore: Send + Sync {
    async fn put(&self, key: &str, bytes: &[u8]) -> Result<()>;
    async fn get(&self, key: &str) -> Result<Vec<u8>>;
    async fn delete(&self, key: &str) -> Result<()>;
}

#[derive(Debug, Clone)]
pub struct FsObjectStore {
    root: std::path::PathBuf,
}

impl FsObjectStore {
    pub fn new(root: impl AsRef<std::path::Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
        }
    }

    /// Resolve a key to a path strictly under `self.root`, rejecting any key
    /// that could escape the root via traversal, absolute paths, or prefixes.
    fn resolve(&self, key: &str) -> Result<std::path::PathBuf> {
        if key.is_empty() {
            return Err(NoemaError::PolicyDenied("empty object key".to_string()));
        }
        if key.contains('\0') || key.contains('\\') {
            return Err(NoemaError::PolicyDenied(format!(
                "object key contains forbidden character: {key:?}"
            )));
        }
        let mut path = self.root.clone();
        for component in std::path::Path::new(key).components() {
            match component {
                std::path::Component::Normal(part) => path.push(part),
                _ => {
                    return Err(NoemaError::PolicyDenied(format!(
                        "object key escapes root: {key:?}"
                    )));
                }
            }
        }
        Ok(path)
    }
}

#[async_trait]
impl ObjectStore for FsObjectStore {
    async fn put(&self, key: &str, bytes: &[u8]) -> Result<()> {
        let path = self.resolve(key)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(path, bytes).await?;
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>> {
        Ok(tokio::fs::read(self.resolve(key)?).await?)
    }

    async fn delete(&self, key: &str) -> Result<()> {
        let path = self.resolve(key)?;
        if path.exists() {
            tokio::fs::remove_file(path).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_traversal_and_absolute_keys() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsObjectStore::new(dir.path());
        for key in ["", "../etc/passwd", "/etc/passwd", "a/../../b", "a\\b"] {
            assert!(
                matches!(store.put(key, b"x").await, Err(NoemaError::PolicyDenied(_))),
                "key should be rejected: {key:?}"
            );
            assert!(
                matches!(store.get(key).await, Err(NoemaError::PolicyDenied(_))),
                "key should be rejected: {key:?}"
            );
            assert!(
                matches!(store.delete(key).await, Err(NoemaError::PolicyDenied(_))),
                "key should be rejected: {key:?}"
            );
        }
    }

    #[tokio::test]
    async fn normal_nested_key_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsObjectStore::new(dir.path());
        let key = "tenants/personal/deep/mem.md";
        store.put(key, b"hello").await.unwrap();
        assert_eq!(store.get(key).await.unwrap(), b"hello");
    }
}
