mod config;
mod maze;
mod navigation;
mod pace;
mod record;
mod report;
mod runner;
mod session;
mod stats;
mod target;
mod trajectory_memory;

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::EnvFilter;

use crate::config::{Cli, Command};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("warn".parse()?))
        .init();
    let cli = Cli::parse();
    match cli.command {
        Command::Latency(args) => runner::run_latency(args).await,
        Command::Load(args) => runner::run_load(args).await,
        Command::MazeRecord(args) => runner::run_maze(args).await,
        Command::MazeSession(args) => session::run(args).await,
    }
}
