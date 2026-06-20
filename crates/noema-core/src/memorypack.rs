use serde::{Deserialize, Serialize};

use crate::ids::{MemoryId, TenantId};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryPack {
    pub tenant_id: TenantId,
    pub memories: Vec<MemoryPackItem>,
    pub subconscious_hints: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryPackItem {
    pub id: MemoryId,
    pub scope: String,
    pub kind: String,
    pub text: Option<String>,
    pub withheld_by_policy: bool,
    pub score: f32,
}

impl MemoryPack {
    pub fn empty(tenant_id: TenantId) -> Self {
        Self {
            tenant_id,
            memories: Vec::new(),
            subconscious_hints: Vec::new(),
        }
    }

    pub fn to_markdown(&self) -> String {
        let mut out = String::from("## Relevant Memories\n");
        for item in &self.memories {
            if item.withheld_by_policy {
                out.push_str(&format!(
                    "- [{}/{}][{}][withheld_by_policy]\n",
                    item.scope, item.kind, item.id
                ));
            } else if let Some(text) = &item.text {
                out.push_str(&format!(
                    "- [{}/{}][{}] {}\n",
                    item.scope, item.kind, item.id, text
                ));
            }
        }
        out.push_str("\n## Subconscious Hints\n");
        for hint in &self.subconscious_hints {
            out.push_str("- ");
            out.push_str(hint);
            out.push('\n');
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memorypack_renders_withheld_markers_without_body() {
        let pack = MemoryPack {
            tenant_id: TenantId::new("personal"),
            memories: vec![MemoryPackItem {
                id: MemoryId::new("mem_sensitive"),
                scope: "project".to_string(),
                kind: "warning".to_string(),
                text: None,
                withheld_by_policy: true,
                score: 0.9,
            }],
            subconscious_hints: vec!["cue: noema -> memory".to_string()],
        };

        let rendered = pack.to_markdown();
        assert!(rendered.contains("[withheld_by_policy]"));
        assert!(!rendered.contains("raw incident"));
    }
}
