use serde::{de::DeserializeOwned, Serialize};

use crate::error::{NoemaError, Result};

pub fn encode<T: Serialize>(frontmatter: &T, body: &str) -> Result<String> {
    let head = serde_json::to_string_pretty(frontmatter)?;
    Ok(format!("---json\n{head}\n---\n{body}\n"))
}

pub fn decode<T: DeserializeOwned>(text: &str) -> Result<(T, String)> {
    let Some(rest) = text.strip_prefix("---json\n") else {
        return Err(NoemaError::InvalidRecord(
            "missing ---json frontmatter".into(),
        ));
    };
    let Some((head, body)) = rest.split_once("\n---\n") else {
        return Err(NoemaError::InvalidRecord("unterminated frontmatter".into()));
    };
    let frontmatter = serde_json::from_str(head)?;
    Ok((frontmatter, body.trim_end_matches('\n').to_string()))
}
