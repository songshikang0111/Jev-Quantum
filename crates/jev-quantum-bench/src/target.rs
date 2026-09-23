use std::time::Duration;

use anyhow::Result;
use jev_quantum_core::protocol::{SystemOneRequest, SystemOneResponse};
use reqwest::Client;

use crate::config::{BenchConfig, TargetKind};
use crate::record::{classify_status, ERR_NETWORK, ERR_OK, ERR_SCHEMA, ERR_TIMEOUT};

#[derive(Debug, Clone)]
pub struct Target {
    pub name: String,
    pub endpoint: String,
    pub model: String,
    pub api_key: Option<String>,
    pub paced: bool,
}

impl Target {
    pub fn from_config(kind: TargetKind, config: &BenchConfig) -> Option<Self> {
        match kind {
            TargetKind::Local => Some(Self {
                name: kind.as_str().to_string(),
                endpoint: format!(
                    "{}/v1/systemone",
                    config.local_base_url.trim_end_matches('/')
                ),
                model: config.local_model.clone(),
                api_key: None,
                paced: false,
            }),
            TargetKind::FloodFill | TargetKind::MemoryRules => Some(Self {
                name: kind.as_str().into(),
                endpoint: "in-process".into(),
                model: if kind == TargetKind::MemoryRules {
                    "spatial-v2-rules"
                } else {
                    "online-flood-fill"
                }
                .into(),
                api_key: None,
                paced: false,
            }),
            TargetKind::Jev
            | TargetKind::JevMemory
            | TargetKind::JevMemoryLong
            | TargetKind::JevMemoryFree
            | TargetKind::JevMemoryNoDistance
            | TargetKind::JevFloodFill => config.gateway_api_key.as_ref().map(|key| Self {
                name: kind.as_str().to_string(),
                endpoint: format!(
                    "{}/v1/systemone",
                    config.gateway_base_url.trim_end_matches('/')
                ),
                model: config.gateway_model.clone(),
                api_key: Some(key.clone()),
                paced: true,
            }),
        }
    }
}

pub fn http_client(timeout: Duration) -> Result<Client> {
    Ok(Client::builder()
        .timeout(timeout)
        .pool_max_idle_per_host(64)
        .tcp_nodelay(true)
        .build()?)
}

#[derive(Debug)]
pub struct TargetResponse {
    pub status: u16,
    pub latency_ns: u64,
    pub bytes: u32,
    pub body: Option<SystemOneResponse>,
    pub error: u8,
}

pub async fn send_request(
    client: &Client,
    target: &Target,
    mut request: SystemOneRequest,
) -> TargetResponse {
    request.model = target.model.clone();
    let mut builder = client.post(&target.endpoint).json(&request);
    if let Some(key) = &target.api_key {
        builder = builder.bearer_auth(key);
    }
    let started = std::time::Instant::now();
    match builder.send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let bytes = resp.bytes().await.unwrap_or_default();
            let parsed = serde_json::from_slice::<SystemOneResponse>(&bytes).ok();
            let error = if !(200..300).contains(&status) {
                classify_status(status)
            } else if parsed.is_none() {
                ERR_SCHEMA
            } else {
                ERR_OK
            };
            TargetResponse {
                status,
                latency_ns: started.elapsed().as_nanos() as u64,
                bytes: bytes.len() as u32,
                body: parsed,
                error,
            }
        }
        Err(err) => TargetResponse {
            status: 0,
            latency_ns: started.elapsed().as_nanos() as u64,
            bytes: 0,
            body: None,
            error: if err.is_timeout() {
                ERR_TIMEOUT
            } else {
                ERR_NETWORK
            },
        },
    }
}
