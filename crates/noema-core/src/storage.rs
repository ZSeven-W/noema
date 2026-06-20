use async_trait::async_trait;

use crate::error::Result;

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

    fn path(&self, key: &str) -> std::path::PathBuf {
        self.root.join(key)
    }
}

#[async_trait]
impl ObjectStore for FsObjectStore {
    async fn put(&self, key: &str, bytes: &[u8]) -> Result<()> {
        let path = self.path(key);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(path, bytes).await?;
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>> {
        Ok(tokio::fs::read(self.path(key)).await?)
    }

    async fn delete(&self, key: &str) -> Result<()> {
        let path = self.path(key);
        if path.exists() {
            tokio::fs::remove_file(path).await?;
        }
        Ok(())
    }
}
