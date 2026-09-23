use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};

#[derive(Debug, Clone, Parser)]
#[command(
    name = "jev-quantum-bench",
    about = "Compare local Quantum against TypeSafe Jev"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    Latency(RunArgs),
    Load(LoadArgs),
    MazeRecord(MazeArgs),
    MazeSession(crate::session::SessionArgs),
}

#[derive(Debug, Clone, Parser)]
pub struct RunArgs {
    #[arg(long, default_value = "local")]
    pub targets: String,
    #[arg(long, default_value_t = 100)]
    pub requests: usize,
    #[arg(long, default_value_t = 1)]
    pub concurrency: usize,
    #[arg(long, default_value_t = 10)]
    pub warmup: usize,
    #[arg(long)]
    pub output: Option<PathBuf>,
    #[arg(long)]
    pub local_base_url: Option<String>,
    #[arg(long)]
    pub local_model: Option<String>,
    #[arg(long)]
    pub gateway_base_url: Option<String>,
    #[arg(long)]
    pub gateway_model: Option<String>,
    #[arg(long, default_value_t = 30_000)]
    pub timeout_ms: u64,
    /// Cap remote TypeSafe Jev calls. `0` disables pacing. Local is never paced.
    #[arg(long, default_value_t = 10.0)]
    pub remote_qps: f64,
    /// Retries for a rate-limited or transiently unavailable remote maze step.
    #[arg(long, default_value_t = 6)]
    pub remote_max_retries: u32,
    /// Initial remote maze retry delay; each retry doubles it, capped at 60 seconds.
    #[arg(long, default_value_t = 1_000)]
    pub remote_retry_base_ms: u64,
}

#[derive(Debug, Clone, Parser)]
pub struct LoadArgs {
    #[command(flatten)]
    pub run: RunArgs,
    /// Required to send high-concurrency traffic to TypeSafe Jev.
    #[arg(long, default_value_t = false)]
    pub enable_remote_load: bool,
}

#[derive(Debug, Clone, Parser)]
pub struct MazeArgs {
    /// Continue a saved maze trajectory; dimensions and seed must match.
    #[arg(long)]
    pub resume_report: Option<PathBuf>,
    #[command(flatten)]
    pub run: RunArgs,
    #[arg(long, default_value_t = 15)]
    pub width: usize,
    #[arg(long, default_value_t = 15)]
    pub height: usize,
    #[arg(long, default_value_t = 7)]
    pub maze_seed: u64,
    #[arg(long, default_value_t = 2_000)]
    pub max_steps: usize,
    /// Wall-clock cap for the whole maze-record run, in seconds. `0` disables. Default 10 minutes.
    #[arg(long, default_value_t = 600)]
    pub max_runtime_secs: u64,
    /// `features` compiles walls, goal vector, visits, and legal moves. `minimal` is the old bare payload.
    #[arg(long, default_value = "features", value_parser = crate::maze::MazeContext::parse)]
    pub context: crate::maze::MazeContext,
}

#[derive(Debug, Clone)]
pub struct BenchConfig {
    pub local_base_url: String,
    pub local_model: String,
    pub gateway_base_url: String,
    pub gateway_model: String,
    pub gateway_api_key: Option<String>,
    pub report_output_dir: PathBuf,
}

impl BenchConfig {
    pub fn load() -> Result<Self> {
        let _ = dotenvy::dotenv();
        Ok(Self {
            local_base_url: env_or("LOCAL_API_BASE_URL", "http://127.0.0.1:3000"),
            local_model: env_or("LOCAL_API_MODEL", "jev-quantum-latest"),
            gateway_base_url: env_or("AI_GATEWAY_BASE_URL", "https://api.typesafe.ai"),
            gateway_model: env_or("AI_GATEWAY_MODEL", "jev-latest"),
            gateway_api_key: std::env::var("AI_GATEWAY_API_KEY")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            report_output_dir: PathBuf::from(env_or("REPORT_OUTPUT_DIR", "reports")),
        })
    }

    pub fn apply_overrides(&mut self, args: &RunArgs) {
        if let Some(url) = &args.local_base_url {
            self.local_base_url = url.clone();
        }
        if let Some(model) = &args.local_model {
            self.local_model = model.clone();
        }
        if let Some(url) = &args.gateway_base_url {
            self.gateway_base_url = url.clone();
        }
        if let Some(model) = &args.gateway_model {
            self.gateway_model = model.clone();
        }
        if let Some(dir) = &args.output {
            self.report_output_dir = dir.clone();
        }
    }

    pub fn describe(&self) -> String {
        format!(
            "local={} model={} | gateway={} model={} key={}",
            self.local_base_url,
            self.local_model,
            self.gateway_base_url,
            self.gateway_model,
            if self.gateway_api_key.is_some() {
                "configured"
            } else {
                "missing"
            }
        )
    }
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| default.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    Local,
    Jev,
    JevMemory,
    JevFloodFill,
    FloodFill,
}

impl TargetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Jev => "jev",
            Self::JevMemory => "jev-memory",
            Self::JevFloodFill => "jev-flood-fill",
            Self::FloodFill => "flood-fill",
        }
    }

    pub fn parse_list(raw: &str) -> Result<Vec<Self>> {
        let mut out = Vec::new();
        for part in raw.split(',') {
            let item = part.trim().to_ascii_lowercase();
            if item.is_empty() {
                continue;
            }
            out.push(match item.as_str() {
                "local" => Self::Local,
                "jev" | "gateway" | "remote" => Self::Jev,
                "jev-memory" => Self::JevMemory,
                "jev-flood-fill" => Self::JevFloodFill,
                "flood-fill" => Self::FloodFill,
                other => bail!(
                    "unknown target '{other}' (use local,jev,flood-fill,jev-memory,jev-flood-fill)"
                ),
            });
        }
        if out.is_empty() {
            bail!("at least one target is required");
        }
        Ok(out)
    }
}

pub fn require_positive(name: &str, value: usize) -> Result<()> {
    if value == 0 {
        anyhow::bail!("{name} must be > 0");
    }
    Ok(())
}

pub fn load_merged(args: &RunArgs) -> Result<BenchConfig> {
    let mut config = BenchConfig::load().context("failed to load .env")?;
    config.apply_overrides(args);
    Ok(config)
}
