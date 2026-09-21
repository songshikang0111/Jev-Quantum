use std::sync::Arc;

use jev_quantum_server::app::{self, AppState};
use jev_quantum_server::config::ServerConfig;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse()?))
        .init();

    let config = ServerConfig::parse();
    let state = Arc::new(AppState::new(config.clone()));
    let app = app::router(Arc::clone(&state));
    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    let addr = listener.local_addr()?;
    tracing::info!(
        %addr,
        model = %config.model_id,
        rng_mode = %config.rng_mode.as_str(),
        backend = %state.engine.backend().as_str(),
        "jev-quantum-server listening"
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutdown signal received");
}
