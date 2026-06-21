use serde::{de::DeserializeOwned, Serialize};

use crate::error::{NoemaError, Result};

pub fn encode<T: Serialize>(frontmatter: &T, body: &str) -> Result<String> {
    let head = serde_json::to_string_pretty(frontmatter)?;
    Ok(format!("---json\n{head}\n---\n{body}"))
}

pub fn decode<T: DeserializeOwned>(text: &str) -> Result<(T, String)> {
    let rest = text
        .strip_prefix("---json\n")
        .ok_or_else(|| NoemaError::InvalidRecord("missing ---json frontmatter".into()))?;
    let (head, body) = rest
        .split_once("\n---\n")
        .ok_or_else(|| NoemaError::InvalidRecord("unterminated frontmatter".into()))?;
    let frontmatter = serde_json::from_str(head)?;
    Ok((frontmatter, body.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_with_trailing_newlines_round_trips() {
        #[derive(serde::Serialize, serde::Deserialize)]
        struct Fm {
            id: String,
        }
        let body = "line one\n\nline two\n\n";
        let encoded = encode(&Fm { id: "x".into() }, body).unwrap();
        let (_fm, decoded): (Fm, String) = decode(&encoded).unwrap();
        assert_eq!(decoded, body);
    }
}
