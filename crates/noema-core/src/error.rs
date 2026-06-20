use thiserror::Error;

#[derive(Debug, Error)]
pub enum NoemaError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("jwt: {0}")]
    Jwt(#[from] jsonwebtoken::errors::Error),
    #[error("toml decode: {0}")]
    TomlDe(#[from] toml::de::Error),
    #[error("toml encode: {0}")]
    TomlSer(#[from] toml::ser::Error),
    #[error("invalid record: {0}")]
    InvalidRecord(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("policy denied: {0}")]
    PolicyDenied(String),
}

pub type Result<T> = std::result::Result<T, NoemaError>;
