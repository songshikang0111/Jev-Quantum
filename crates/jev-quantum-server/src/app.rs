use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use axum::extract::State;
use axum::http::{header, StatusCode, Uri};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use jev_quantum_core::protocol::{models_catalog, SystemOneRequest};
use jev_quantum_core::DecisionEngine;
use rust_embed::RustEmbed;
use serde::Serialize;
use tower_http::limit::RequestBodyLimitLayer;

use crate::config::ServerConfig;
use crate::metrics::HttpMetrics;

#[derive(RustEmbed)]
#[folder = "../../web"]
struct Assets;

pub struct AppState {
    pub engine: DecisionEngine,
    pub metrics: HttpMetrics,
    pub config: ServerConfig,
}

impl AppState {
    pub fn new(config: ServerConfig) -> Self {
        Self {
            engine: DecisionEngine::new(config.engine_config()),
            metrics: HttpMetrics::default(),
            config,
        }
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    let limit = state.config.body_limit;
    Router::new()
        .route("/v1/systemone", post(systemone))
        .route("/v1/models", get(models))
        .route("/health", get(health))
        .route("/metrics", get(metrics))
        .route("/demo", get(demo_redirect))
        .route("/demo/", get(demo_index))
        .route("/demo/{*path}", get(demo_asset))
        .layer(RequestBodyLimitLayer::new(limit))
        .with_state(state)
}

async fn systemone(
    State(state): State<Arc<AppState>>,
    Json(request): Json<SystemOneRequest>,
) -> Response {
    let started = Instant::now();
    match state.engine.decide(&request) {
        Ok(response) => {
            state.metrics.record(started, false);
            (StatusCode::OK, Json(response)).into_response()
        }
        Err(error) => {
            state.metrics.record(started, true);
            let status = match error {
                jev_quantum_core::ProtocolError::MissingModel
                | jev_quantum_core::ProtocolError::EmptyQuestions => {
                    StatusCode::UNPROCESSABLE_ENTITY
                }
                jev_quantum_core::ProtocolError::InvalidChoiceCardinality(_)
                | jev_quantum_core::ProtocolError::InvalidScoreCardinality(_) => {
                    StatusCode::UNPROCESSABLE_ENTITY
                }
            };
            (status, Json(error.into_api_error())).into_response()
        }
    }
}

async fn models(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(models_catalog(state.engine.model_id()))
}

#[derive(Serialize)]
struct HealthBody {
    status: &'static str,
    model: String,
    backend: &'static str,
    rng_mode: &'static str,
}

async fn health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(HealthBody {
        status: "ok",
        model: state.engine.model_id().to_string(),
        backend: state.engine.backend().as_str(),
        rng_mode: state.engine.mode().as_str(),
    })
}

async fn metrics(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let rng = state.engine.stats();
    let body = state.metrics.render(
        state.engine.backend().as_str(),
        state.engine.mode().as_str(),
        rng.hits,
        rng.fallbacks,
        rng.refill_ns,
        rng.refills,
    );
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        body,
    )
}

async fn demo_redirect() -> Redirect {
    Redirect::permanent("/demo/")
}

async fn demo_index() -> Response {
    serve_asset("index.html")
}

async fn demo_asset(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches("/demo/");
    if path.is_empty() {
        return serve_asset("index.html");
    }
    serve_asset(path)
}

fn serve_asset(path: &str) -> Response {
    if path.contains("..") || Path::new(path).is_absolute() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let disk = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../web")
        .join(path);
    if let Ok(bytes) = std::fs::read(&disk) {
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, mime_for(path))],
            bytes,
        )
            .into_response();
    }
    if let Some(file) = Assets::get(path) {
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, mime_for(path))],
            file.data.into_owned(),
        )
            .into_response();
    }
    if path == "index.html" {
        return Html("<p>demo assets missing</p>").into_response();
    }
    StatusCode::NOT_FOUND.into_response()
}

fn mime_for(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "application/javascript; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }
}
