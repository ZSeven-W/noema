use axum::{routing::get, Json, Router};
use serde_json::{json, Value};

// NOTE: this server is an enterprise *scaffold*. The `/status` route does NOT
// authenticate callers: `noema_core::identity::verify_principal` exists but is
// not yet wired into request handling, and no handler enforces ACL/policy.
// `auth_enforced`/`acl_enforced` are reported as `false` so the payload does not
// advertise a trust boundary that is not actually enforced at runtime.
pub fn status_payload() -> Value {
    json!({
        "service": "noema-server",
        "trust_boundary": "signed-principal",
        "auth_enforced": false,
        "acl_enforced": false
    })
}

async fn status() -> Json<Value> {
    Json(status_payload())
}

#[tokio::main]
async fn main() {
    let app = Router::new().route("/status", get(status));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8765")
        .await
        .expect("bind noema-server");
    axum::serve(listener, app)
        .await
        .expect("serve noema-server");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn status_payload_reports_enterprise_boundary() {
        let payload = status_payload();
        assert_eq!(payload["service"], "noema-server");
        assert_eq!(payload["trust_boundary"], "signed-principal");
    }
}
