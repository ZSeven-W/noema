use std::path::Path;

use crate::error::Result;
use crate::frontmatter;
use crate::lock::atomic_write;
use crate::memory::MemoryRecord;

pub fn write_memory(path: &Path, memory: &MemoryRecord) -> Result<()> {
    let mut frontmatter = memory.clone();
    let body = std::mem::take(&mut frontmatter.body);
    let encoded = frontmatter::encode(&frontmatter, &body)?;
    atomic_write(path, encoded.as_bytes())
}

pub fn read_memory(path: &Path) -> Result<MemoryRecord> {
    let text = std::fs::read_to_string(path)?;
    let (mut frontmatter, body): (MemoryRecord, String) = frontmatter::decode(&text)?;
    frontmatter.body = body;
    Ok(frontmatter)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{MemoryId, TenantId, UserId};
    use crate::memory::MemoryRecord;

    #[test]
    fn markdown_memory_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mem.md");
        let memory = MemoryRecord::new_user_preference(
            MemoryId::new("mem_test"),
            TenantId::new("personal"),
            UserId::new("kay"),
            "Prefer concise answers.",
        );
        write_memory(&path, &memory).unwrap();
        let loaded = read_memory(&path).unwrap();
        assert_eq!(loaded.id, memory.id);
        assert_eq!(loaded.body, "Prefer concise answers.");
        assert_eq!(loaded.schema_version, 1);
    }
}
