use std::sync::Arc;

use noema_core::api::NoemaEngine;
use noema_core::config::NoemaConfig;
use noema_core::ids::UserId;
use noema_mcp::{personal_principal, NoemaTools};
use rmcp::ServiceExt;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lists_tools_and_recalls() {
    // Isolated temp directory so we never touch the real ~/.agent-memory.
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("NOEMA_ROOT", dir.path());

    let cfg = NoemaConfig::default();
    let engine = Arc::new(NoemaEngine::from_config(&cfg).unwrap());
    engine
        .init_personal(&UserId::new(&cfg.tenant.default_user_id))
        .unwrap();
    let tools = NoemaTools::new(engine, personal_principal(&cfg));

    // In-process duplex transport: server and client share a tokio duplex pair.
    // DuplexStream implements AsyncRead + AsyncWrite, which maps to
    // TransportAdapterAsyncCombinedRW in rmcp.
    let (client_stream, server_stream) = tokio::io::duplex(65536);

    // The MCP handshake (initialize / initialized) must run concurrently:
    // the client sends initialize and blocks until the server responds,
    // while the server blocks until it receives initialize.  Use tokio::join!
    // to drive both futures simultaneously.
    let (server_result, client_result) =
        tokio::join!(tools.serve(server_stream), ().serve(client_stream),);
    let server = server_result.unwrap();
    let client = client_result.unwrap();

    // Verify that the expected tools are advertised.
    let listed = client.list_tools(Default::default()).await.unwrap();
    let names: Vec<String> = listed.tools.iter().map(|t| t.name.to_string()).collect();
    assert!(
        names.iter().any(|n| n == "noema_recall"),
        "expected noema_recall in tool list, got: {names:?}"
    );
    assert!(
        names.iter().any(|n| n == "noema_review_decide"),
        "expected noema_review_decide in tool list, got: {names:?}"
    );

    // Clean shutdown.
    client.cancel().await.ok();
    server.cancel().await.ok();
}
