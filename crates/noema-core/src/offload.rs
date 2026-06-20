use serde::{Deserialize, Serialize};

use crate::ids::{MemoryId, TenantId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColdPointer {
    pub tenant_id: TenantId,
    pub memory_id: MemoryId,
    pub key: String,
    pub compression: String,
    pub encryption: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeletionManifest {
    pub tenant_id: TenantId,
    pub erased_memory_ids: Vec<MemoryId>,
}

impl DeletionManifest {
    pub fn is_erased(&self, id: &MemoryId) -> bool {
        self.erased_memory_ids.iter().any(|erased| erased == id)
    }
}

pub fn compressed_payload(bytes: &[u8]) -> crate::error::Result<Vec<u8>> {
    Ok(zstd::encode_all(bytes, 3)?)
}

pub fn decompressed_payload(bytes: &[u8]) -> crate::error::Result<Vec<u8>> {
    Ok(zstd::decode_all(bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deletion_manifest_filters_restore_records() {
        let manifest = DeletionManifest {
            tenant_id: TenantId::new("personal"),
            erased_memory_ids: vec![MemoryId::new("mem_a")],
        };
        assert!(manifest.is_erased(&MemoryId::new("mem_a")));
        assert!(!manifest.is_erased(&MemoryId::new("mem_b")));
    }

    #[test]
    fn compression_roundtrips_payload() {
        let compressed = compressed_payload(b"memory body").unwrap();
        let plain = decompressed_payload(&compressed).unwrap();
        assert_eq!(plain, b"memory body");
    }
}
