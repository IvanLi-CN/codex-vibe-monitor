#[path = "../oauth_bridge.rs"]
mod oauth_bridge;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    oauth_bridge::run_fixed_oauth_bridge_server().await
}
