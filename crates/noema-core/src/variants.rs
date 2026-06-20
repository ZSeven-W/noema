use serde::{Deserialize, Serialize};

use crate::ids::MemoryId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SafeVariantMode {
    Redacted,
    SummaryOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SafeVariant {
    pub memory_id: MemoryId,
    pub mode: SafeVariantMode,
    pub text: String,
    pub generated_by: String,
}

pub fn select_variant<'a>(
    variants: &'a [SafeVariant],
    memory_id: &MemoryId,
    mode: SafeVariantMode,
) -> Option<&'a SafeVariant> {
    variants
        .iter()
        .find(|variant| &variant.memory_id == memory_id && variant.mode == mode)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacted_variant_never_contains_raw_body() {
        let variant = SafeVariant {
            memory_id: MemoryId::new("mem_a"),
            mode: SafeVariantMode::Redacted,
            text: "Customer [customer] reported incident [incident].".to_string(),
            generated_by: "reviewer".to_string(),
        };
        assert!(!variant.text.contains("Acme"));
    }

    #[test]
    fn variant_selection_matches_mode() {
        let memory_id = MemoryId::new("mem_a");
        let variants = vec![SafeVariant {
            memory_id: memory_id.clone(),
            mode: SafeVariantMode::SummaryOnly,
            text: "A safe summary.".to_string(),
            generated_by: "reviewer".to_string(),
        }];
        assert_eq!(
            select_variant(&variants, &memory_id, SafeVariantMode::SummaryOnly)
                .unwrap()
                .text,
            "A safe summary."
        );
    }
}
