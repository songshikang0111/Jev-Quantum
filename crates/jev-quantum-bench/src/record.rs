use serde::{Deserialize, Serialize};

pub const ERR_OK: u8 = 0;
pub const ERR_TIMEOUT: u8 = 1;
pub const ERR_SCHEMA: u8 = 2;
pub const ERR_HTTP: u8 = 3;
pub const ERR_AUTH: u8 = 4;
pub const ERR_RATE_LIMIT: u8 = 5;
pub const ERR_NETWORK: u8 = 6;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct RequestRecord {
    pub offset_ns: u64,
    pub latency_ns: u64,
    #[serde(default)]
    pub throttle_ns: u64,
    pub status: u16,
    pub action: u8,
    pub error: u8,
    pub bytes: u32,
}

impl RequestRecord {
    pub fn error_name(code: u8) -> &'static str {
        match code {
            ERR_OK => "ok",
            ERR_TIMEOUT => "timeout",
            ERR_SCHEMA => "schema",
            ERR_HTTP => "http",
            ERR_AUTH => "auth",
            ERR_RATE_LIMIT => "rate_limit",
            ERR_NETWORK => "network",
            _ => "unknown",
        }
    }
}

pub fn classify_status(status: u16) -> u8 {
    match status {
        200..=299 => ERR_OK,
        401 | 403 => ERR_AUTH,
        429 => ERR_RATE_LIMIT,
        _ => ERR_HTTP,
    }
}

pub fn action_from_choice(label: &str) -> u8 {
    match label.to_ascii_uppercase().as_str() {
        "UP" | "NORTH" => 0,
        "RIGHT" | "EAST" => 1,
        "DOWN" | "SOUTH" => 2,
        "LEFT" | "WEST" => 3,
        _ => 255,
    }
}

pub fn action_name(code: u8) -> &'static str {
    match code {
        0 => "UP",
        1 => "RIGHT",
        2 => "DOWN",
        3 => "LEFT",
        _ => "UNKNOWN",
    }
}
