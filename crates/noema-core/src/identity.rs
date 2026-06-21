use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

use crate::error::{NoemaError, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrincipalClaims {
    pub tenant_id: String,
    pub user_id: String,
    pub groups: Vec<String>,
    pub roles: Vec<String>,
    pub host: String,
    pub clearance: String,
    pub exp: u64,
    pub iss: String,
    pub aud: String,
    pub sub: String,
}

pub fn sign_principal(claims: &PrincipalClaims, secret: &[u8]) -> Result<String> {
    if secret.len() < 32 {
        return Err(NoemaError::PolicyDenied("hmac secret too short".into()));
    }
    Ok(encode(
        &Header::default(),
        claims,
        &EncodingKey::from_secret(secret),
    )?)
}

pub fn verify_principal(token: &str, secret: &[u8]) -> Result<PrincipalClaims> {
    if secret.len() < 32 {
        return Err(NoemaError::PolicyDenied("hmac secret too short".into()));
    }
    let mut v = Validation::new(Algorithm::HS256);
    v.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
    v.set_issuer(&["noema"]);
    v.set_audience(&["noema"]);
    let data = decode::<PrincipalClaims>(token, &DecodingKey::from_secret(secret), &v)?;
    Ok(data.claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"test-secret-test-secret-test-secret!!";

    fn sample_claims() -> PrincipalClaims {
        PrincipalClaims {
            tenant_id: "acme".to_string(),
            user_id: "kay".to_string(),
            groups: vec!["eng".to_string()],
            roles: vec!["reviewer".to_string()],
            host: "noema-server".to_string(),
            clearance: "confidential".to_string(),
            exp: 4_102_444_800,
            iss: "noema".into(),
            aud: "noema".into(),
            sub: "kay".into(),
        }
    }

    #[test]
    fn signed_principal_roundtrips() {
        let claims = sample_claims();
        let token = sign_principal(&claims, SECRET).unwrap();
        let verified = verify_principal(&token, SECRET).unwrap();
        assert_eq!(verified.tenant_id, "acme");
        assert_eq!(verified.roles, vec!["reviewer"]);
    }

    #[test]
    fn verify_rejects_wrong_audience() {
        let mut claims = sample_claims();
        claims.aud = "evil".into();
        let token = sign_principal(&claims, SECRET).unwrap();
        assert!(verify_principal(&token, SECRET).is_err());
    }

    #[test]
    fn sign_rejects_short_secret() {
        let claims = sample_claims();
        assert!(sign_principal(&claims, b"short").is_err());
    }

    #[test]
    fn verify_rejects_expired_token() {
        // exp in the past (year 2001) — token must be rejected
        let mut claims = sample_claims();
        claims.exp = 1_000_000_000;
        let token = sign_principal(&claims, SECRET).unwrap();
        assert!(verify_principal(&token, SECRET).is_err());
    }

    #[test]
    fn verify_rejects_tampered_signature() {
        // flip one character in the signature segment (after the last '.')
        let mut token = sign_principal(&sample_claims(), SECRET).unwrap();
        let last = token.pop().unwrap();
        token.push(if last == 'a' { 'b' } else { 'a' });
        assert!(verify_principal(&token, SECRET).is_err());
    }
}
