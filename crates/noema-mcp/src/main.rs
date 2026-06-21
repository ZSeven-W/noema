use std::sync::Arc;

use noema_core::api::NoemaEngine;
use noema_core::config::NoemaConfig;
use noema_core::ids::UserId;
use noema_mcp::{personal_principal, NoemaTools};
use rmcp::{transport::stdio, ServiceExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = NoemaConfig::load_or_default()?;
    let engine = Arc::new(NoemaEngine::from_config(&cfg)?);
    engine.init_personal(&UserId::new(&cfg.tenant.default_user_id))?;
    let tools = NoemaTools::new(engine, personal_principal(&cfg));
    let service = tools.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
