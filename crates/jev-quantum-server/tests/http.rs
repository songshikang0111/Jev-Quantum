use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use clap::Parser;
use http_body_util::BodyExt;
use jev_quantum_server::app::{router, AppState};
use jev_quantum_server::config::ServerConfig;
use serde_json::{json, Value};
use tower::ServiceExt;

fn test_state() -> Arc<AppState> {
    let mut config = ServerConfig::parse_from(["jev-quantum-server", "--seed", "1"]);
    config.bind = "127.0.0.1:0".parse().unwrap();
    Arc::new(AppState::new(config))
}

async fn call(app: axum::Router, req: Request<Body>) -> (StatusCode, Value) {
    let response = app.oneshot(req).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or(Value::String(String::from_utf8_lossy(&bytes).into_owned()))
    };
    (status, json)
}

#[tokio::test]
async fn systemone_returns_typed_answers() {
    let app = router(test_state());
    let (status, body) = call(
        app,
        Request::post("/v1/systemone")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&json!({
                    "model": "jev-quantum-latest",
                    "state": "I was charged twice.",
                    "questions": {
                        "refund": { "type": "noul", "instructions": "refund?" },
                        "queue": {
                            "type": "choice",
                            "criteria": { "billing": "pay", "tech": "bug" }
                        }
                    }
                }))
                .unwrap(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["model"], "jev-quantum-latest");
    assert!(body["answers"]["refund"]["noul"].as_f64().is_some());
    assert!(body["answers"]["queue"]["choice"].as_str().is_some());
}

#[tokio::test]
async fn empty_questions_are_422() {
    let app = router(test_state());
    let (status, body) = call(
        app,
        Request::post("/v1/systemone")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&json!({
                    "model": "jev-quantum-latest",
                    "state": "x",
                    "questions": {}
                }))
                .unwrap(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"]["code"], "invalid_request");
}

#[tokio::test]
async fn health_and_models() {
    let state = test_state();
    let app = router(Arc::clone(&state));
    let (status, body) = call(app, Request::get("/health").body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");

    let app = router(state);
    let (status, body) = call(app, Request::get("/v1/models").body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"][0]["id"], "jev-quantum-latest");
}

#[tokio::test]
async fn metrics_and_demo_are_served() {
    let app = router(test_state());
    let response = app
        .oneshot(Request::get("/metrics").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let text = String::from_utf8(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(text.contains("jev_quantum_requests_total"));

    let app = router(test_state());
    let response = app
        .oneshot(Request::get("/demo").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PERMANENT_REDIRECT);
    assert_eq!(response.headers()["location"], "/demo/");
    let response = router(test_state())
        .oneshot(Request::get("/demo/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[test]
fn demo_javascript_never_calls_fetch() {
    let js = include_str!("../../../web/app.js");
    assert!(
        !js.contains("fetch("),
        "maze replay must stay offline and must not call fetch"
    );
}

#[tokio::test]
async fn concurrent_requests_succeed() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = test_state();
    tokio::spawn(async move {
        axum::serve(listener, router(state)).await.unwrap();
    });

    let client = reqwest::Client::new();
    let payload = json!({
        "model": "jev-quantum-latest",
        "state": "concurrent",
        "questions": { "ok": { "type": "noul" } }
    });
    let mut joins = Vec::new();
    for _ in 0..32 {
        let client = client.clone();
        let payload = payload.clone();
        joins.push(tokio::spawn(async move {
            client
                .post(format!("http://{addr}/v1/systemone"))
                .json(&payload)
                .send()
                .await
                .unwrap()
                .status()
        }));
    }
    for join in joins {
        assert_eq!(join.await.unwrap(), reqwest::StatusCode::OK);
    }
}
