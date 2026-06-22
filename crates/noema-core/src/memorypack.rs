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
            if let Some(raw) = &item.text {
                // Strip newlines so a memory body cannot forge markdown structure
                // or inject extra lines into the prompt.
                let text = raw.replace(['\n', '\r'], " ");
                out.push_str(&format!(
                    "- [{}/{}][{}] {}\n",
                    item.scope, item.kind, item.id, text
                ));
            }
        }
        out.push_str("\n## Subconscious Hints\n");
        for hint in &self.subconscious_hints {
            let hint = hint.replace(['\n', '\r'], " ");
            out.push_str("- ");
            out.push_str(&hint);
            out.push('\n');
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memorypack_omits_items_without_body() {
        let pack = MemoryPack {
            tenant_id: TenantId::new("personal"),
            memories: vec![MemoryPackItem {
                id: MemoryId::new("mem_bodyless"),
                scope: "project".to_string(),
                kind: "warning".to_string(),
                text: None,
                score: 0.9,
            }],
            subconscious_hints: vec!["cue: noema -> memory".to_string()],
        };

        let rendered = pack.to_markdown();
        assert!(!rendered.contains("mem_bodyless"));
    }

    #[test]
    fn memorypack_sanitizes_newlines_in_body() {
        let pack = MemoryPack {
            tenant_id: TenantId::new("personal"),
            memories: vec![MemoryPackItem {
                id: MemoryId::new("mem_inject"),
                scope: "user".to_string(),
                kind: "preference".to_string(),
                text: Some("line1\n## Injected".to_string()),
                score: 0.5,
            }],
            subconscious_hints: Vec::new(),
        };

        let rendered = pack.to_markdown();
        let item_line = rendered
            .lines()
            .find(|line| line.contains("mem_inject"))
            .expect("item line present");
        assert!(item_line.contains("line1 ## Injected"));
        assert!(!item_line.contains('\n'));
        // The forged header must not start its own markdown line.
        assert!(!rendered.contains("\n## Injected"));
    }
}
