use std::sync::Arc;

use axum::{
    extract::Request,
    http::StatusCode,
    middleware::{self, Next},
    response::Response,
    routing::get,
    Json, Router,
};
use http::HeaderMap;
use noema_core::{
    config::NoemaConfig,
    identity::{verify_principal, PrincipalClaims},
    ids::{GroupId, HostId, TenantId, UserId},
    sensitivity::{DataClassClearance, Principal, SensitivityLevel},
};
use noema_mcp::{personal_principal, NoemaTools};
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpService,
};
use serde_json::{json, Value};

// --------------------------------------------------------------------------
// /status payload
// --------------------------------------------------------------------------

pub fn status_payload() -> Value {
    json!({
        "service": "noema-server",
        "trust_boundary": "signed-principal",
        "auth_enforced": true
    })
}

async fn status() -> Json<Value> {
    Json(status_payload())
}

// --------------------------------------------------------------------------
// JWT → Principal conversion
// --------------------------------------------------------------------------

/// Extract and verify the Bearer JWT from request headers.
/// Returns `None` when the `Authorization` header is absent or the token is invalid.
pub fn principal_from_headers(headers: &HeaderMap, secret: &[u8]) -> Option<Principal> {
    let raw = headers.get("authorization")?.to_str().ok()?;
    let token = raw.strip_prefix("Bearer ")?;
    let claims: PrincipalClaims = verify_principal(token, secret).ok()?;
    Some(claims_to_principal(claims))
}

fn claims_to_principal(c: PrincipalClaims) -> Principal {
    Principal {
        tenant_id: TenantId::new(c.tenant_id),
        user_id: UserId::new(c.user_id),
        groups: c.groups.into_iter().map(GroupId::new).collect(),
        host: HostId::new(c.host),
        roles: c.roles,
        clearance: parse_clearance(&c.clearance),
        data_class_clearances: Vec::<DataClassClearance>::new(),
    }
}

fn parse_clearance(s: &str) -> SensitivityLevel {
    match s {
        "public" => SensitivityLevel::Public,
        "confidential" => SensitivityLevel::Confidential,
        "restricted" => SensitivityLevel::Restricted,
        "secret" => SensitivityLevel::Secret,
        _ => SensitivityLevel::Internal,
    }
}

// --------------------------------------------------------------------------
// Auth middleware: reject /mcp calls that lack a valid Bearer token.
// --------------------------------------------------------------------------

/// axum middleware that enforces Bearer authentication on the `/mcp` routes.
///
/// A `Principal` is inserted into `req.extensions_mut()` on success so that
/// rmcp's `StreamableHttpService` can forward it through `http::request::Parts`
/// into `RequestContext.extensions` where `NoemaTools::principal_for` reads it.
///
/// Returns 401 when the `Authorization` header is absent or the JWT is invalid.
async fn bearer_auth(
    axum::extract::State(secret): axum::extract::State<Arc<Vec<u8>>>,
    mut req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    match principal_from_headers(req.headers(), &secret) {
        Some(principal) => {
            req.extensions_mut().insert(principal);
            Ok(next.run(req).await)
        }
        None => Err(StatusCode::UNAUTHORIZED),
    }
}

// --------------------------------------------------------------------------
// main
// --------------------------------------------------------------------------

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = NoemaConfig::load_or_default()?;
    let engine = Arc::new(noema_core::api::NoemaEngine::from_config(&cfg)?);

    // HMAC secret: must be ≥32 bytes (enforced by identity::sign/verify_principal).
    let secret: Arc<Vec<u8>> = Arc::new(
        std::env::var("NOEMA_HMAC_SECRET")
            .unwrap_or_default()
            .into_bytes(),
    );

    // Fallback principal for non-HTTP transports; never reached on the HTTP
    // MCP path because the auth middleware enforces a valid token first.
    let anon = personal_principal(&cfg);

    let factory_engine = engine.clone();
    let factory_anon = anon.clone();
    let mcp = StreamableHttpService::new(
        move || {
            Ok(NoemaTools::new(
                factory_engine.clone(),
                factory_anon.clone(),
            ))
        },
        Arc::new(LocalSessionManager::default()),
        // Allow any Host header so the service works behind a reverse proxy.
        rmcp::transport::streamable_http_server::StreamableHttpServerConfig::default()
            .disable_allowed_hosts(),
    );

    // Build the /mcp sub-router: nest the raw MCP service, then apply auth.
    let mcp_router: Router = Router::new().nest_service("/", mcp);
    let protected_mcp =
        mcp_router.route_layer(middleware::from_fn_with_state(secret.clone(), bearer_auth));

    let app = Router::new()
        .route("/status", get(status))
        .nest("/mcp", protected_mcp);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:8765").await?;
    axum::serve(listener, app).await?;
    Ok(())
}

// --------------------------------------------------------------------------
// Tests
// --------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_bearer_is_unauthorized() {
        assert!(principal_from_headers(
            &http::HeaderMap::new(),
            b"secret-secret-secret-secret-secret!!"
        )
        .is_none());
    }

    #[test]
    fn valid_jwt_yields_principal() {
        let secret = b"secret-secret-secret-secret-secret!!";
        let claims = noema_core::identity::PrincipalClaims {
            tenant_id: "acme".into(),
            user_id: "kay".into(),
            groups: vec![],
            roles: vec!["reviewer".into()],
            host: "noema-server".into(),
            clearance: "internal".into(),
            iss: "noema".into(),
            aud: "noema".into(),
            sub: "kay".into(),
            exp: 4_102_444_800,
        };
        let token = noema_core::identity::sign_principal(&claims, secret).unwrap();
        let mut headers = http::HeaderMap::new();
        headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
        let principal = principal_from_headers(&headers, secret).unwrap();
        assert_eq!(principal.user_id.as_str(), "kay");
    }

    #[test]
    fn status_payload_advertises_auth_enforced() {
        let payload = status_payload();
        assert_eq!(payload["service"], "noema-server");
        assert_eq!(payload["trust_boundary"], "signed-principal");
        assert_eq!(payload["auth_enforced"], true);
    }
}
