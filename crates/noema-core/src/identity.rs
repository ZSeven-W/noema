use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

use crate::error::Result;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrincipalClaims {
    pub tenant_id: String,
    pub user_id: String,
    pub groups: Vec<String>,
    pub roles: Vec<String>,
    pub host: String,
    pub clearance: String,
    pub exp: u64,
}

pub fn sign_principal(claims: &PrincipalClaims, secret: &[u8]) -> Result<String> {
    Ok(encode(
        &Header::default(),
        claims,
        &EncodingKey::from_secret(secret),
    )?)
}

pub fn verify_principal(token: &str, secret: &[u8]) -> Result<PrincipalClaims> {
    let data = decode::<PrincipalClaims>(
        token,
        &DecodingKey::from_secret(secret),
        &Validation::default(),
    )?;
    Ok(data.claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_principal_roundtrips() {
        let claims = PrincipalClaims {
            tenant_id: "acme".to_string(),
            user_id: "kay".to_string(),
            groups: vec!["eng".to_string()],
            roles: vec!["reviewer".to_string()],
            host: "noema-server".to_string(),
            clearance: "confidential".to_string(),
            exp: 4_102_444_800,
        };
        let token = sign_principal(&claims, b"test-secret").unwrap();
        let verified = verify_principal(&token, b"test-secret").unwrap();
        assert_eq!(verified.tenant_id, "acme");
        assert_eq!(verified.roles, vec!["reviewer"]);
    }
}
