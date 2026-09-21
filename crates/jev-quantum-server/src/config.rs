use std::net::SocketAddr;

use clap::Parser;
use jev_quantum_core::{EngineConfig, RngMode};

#[derive(Debug, Clone, Parser)]
#[command(
    name = "jev-quantum-server",
    about = "System -1 Jev-compatible decision server"
)]
pub struct ServerConfig {
    /// Listen address. Defaults to localhost for safety.
    #[arg(long, env = "JEV_QUANTUM_BIND", default_value = "127.0.0.1:3000")]
    pub bind: SocketAddr,

    /// `direct` (usually fastest for single decisions) or `buffered`.
    #[arg(long, env = "JEV_QUANTUM_RNG_MODE", default_value = "direct", value_parser = parse_rng_mode)]
    pub rng_mode: RngMode,

    #[arg(long, env = "JEV_QUANTUM_SEED")]
    pub seed: Option<u64>,

    #[arg(long, env = "JEV_QUANTUM_CHUNK_LEN", default_value_t = 4096)]
    pub chunk_len: usize,

    #[arg(long, env = "JEV_QUANTUM_CHUNK_COUNT", default_value_t = 16)]
    pub chunk_count: usize,

    #[arg(long, env = "JEV_QUANTUM_LOW_WATERMARK", default_value_t = 1024)]
    pub low_watermark: usize,

    #[arg(long, env = "JEV_QUANTUM_REFILL_THREADS", default_value_t = 1)]
    pub refill_threads: usize,

    #[arg(long, env = "JEV_QUANTUM_MODEL", default_value = "jev-quantum-latest")]
    pub model_id: String,

    /// Maximum JSON body size in bytes.
    #[arg(long, env = "JEV_QUANTUM_BODY_LIMIT", default_value_t = 1_048_576)]
    pub body_limit: usize,
}

impl ServerConfig {
    pub fn parse() -> Self {
        <Self as Parser>::parse()
    }

    pub fn engine_config(&self) -> EngineConfig {
        EngineConfig {
            mode: self.rng_mode,
            seed: self.seed.unwrap_or_else(jev_quantum_core::process_seed),
            chunk_len: self.chunk_len,
            chunk_count: self.chunk_count,
            low_watermark: self.low_watermark,
            refill_threads: self.refill_threads,
            model_id: self.model_id.clone(),
        }
    }
}

fn parse_rng_mode(value: &str) -> Result<RngMode, String> {
    RngMode::parse(value).ok_or_else(|| format!("unknown rng mode '{value}' (use direct|buffered)"))
}
