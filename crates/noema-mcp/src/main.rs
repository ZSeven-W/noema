use anyhow::Result;
use serde_json::json;
use std::io::{self, Read};

fn main() -> Result<()> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let request: serde_json::Value = serde_json::from_str(input.trim())?;
    let id = request.get("id").cloned().unwrap_or(json!(null));
    let method = request
        .get("method")
        .and_then(|value| value.as_str())
        .unwrap_or("");

    let result = match method {
        "tools/list" => tools_list(),
        "tools/call" => tools_call(request.get("params").cloned().unwrap_or(json!({}))),
        _ => json!({"error": format!("unsupported method: {method}")}),
    };

    println!("{}", json!({"jsonrpc":"2.0","id":id,"result":result}));
    Ok(())
}

fn tools_list() -> serde_json::Value {
    json!({
        "tools": [
            {"name": "noema_recall", "readOnlyHint": true},
            {"name": "noema_submit_candidate", "readOnlyHint": false},
            {"name": "noema_remember", "readOnlyHint": false},
            {"name": "noema_review_list", "readOnlyHint": true},
            {"name": "noema_review_decide", "readOnlyHint": false},
            {"name": "noema_search", "readOnlyHint": true},
            {"name": "noema_explain", "readOnlyHint": true},
            {"name": "noema_policy_get", "readOnlyHint": true},
            {"name": "noema_policy_set", "readOnlyHint": false},
            {"name": "noema_status", "readOnlyHint": true}
        ]
    })
}

fn tools_call(params: serde_json::Value) -> serde_json::Value {
    let name = params
        .get("name")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
    match name {
        "noema_status" => json!({"tenant":"personal","mode":"local","ok":true}),
        "noema_policy_get" => json!({"write":"review","auto_accept_max_sensitivity":"internal"}),
        "noema_policy_set" => json!({"ok": true, "audited": true}),
        "noema_recall" | "noema_search" => {
            json!({"query": arguments.get("query").cloned().unwrap_or(json!("")), "memories": []})
        }
        "noema_explain" => {
            json!({"memory_id": arguments.get("memory_id").cloned().unwrap_or(json!("")), "explanation": []})
        }
        "noema_submit_candidate" | "noema_remember" => {
            json!({"ok": true, "route": "pending_review"})
        }
        "noema_review_list" => json!({"pending": []}),
        "noema_review_decide" => json!({"ok": true, "audited": true}),
        _ => json!({"error": format!("unsupported tool: {name}")}),
    }
}
