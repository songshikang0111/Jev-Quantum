use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use crate::maze::{Maze, MazeTrajectory};
use crate::record::RequestRecord;
use crate::stats::{latency_samples, summarize, TargetStats};

pub const SCHEMA_VERSION: u32 = 1;
pub const SAMPLE_LIMIT: usize = 2048;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryReport {
    pub schema_version: u32,
    pub run_id: String,
    pub scenario: String,
    pub started_at: String,
    pub finished_at: String,
    pub config: serde_json::Value,
    pub targets: Vec<TargetReport>,
    pub maze: Option<Maze>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetReport {
    pub name: String,
    pub model: String,
    pub endpoint: String,
    pub stats: TargetStats,
    pub latency_samples: Vec<u64>,
    pub trajectory: Option<MazeTrajectory>,
}

#[derive(Debug, Clone)]
pub struct TargetRun {
    pub name: String,
    pub model: String,
    pub endpoint: String,
    pub records: Vec<RequestRecord>,
    pub wall_ns: u64,
    pub trajectory: Option<MazeTrajectory>,
}

#[derive(Debug, Clone)]
pub struct WrittenPaths {
    pub summary: PathBuf,
    pub requests: PathBuf,
}

pub fn build_summary(
    scenario: &str,
    started_at: DateTime<Utc>,
    config: serde_json::Value,
    runs: &[TargetRun],
    maze: Option<Maze>,
) -> SummaryReport {
    let finished_at = Utc::now();
    SummaryReport {
        schema_version: SCHEMA_VERSION,
        run_id: started_at.format("%Y%m%dT%H%M%S").to_string(),
        scenario: scenario.to_string(),
        started_at: started_at.to_rfc3339_opts(SecondsFormat::Secs, true),
        finished_at: finished_at.to_rfc3339_opts(SecondsFormat::Secs, true),
        config,
        targets: runs
            .iter()
            .map(|run| TargetReport {
                name: run.name.clone(),
                model: run.model.clone(),
                endpoint: redact_endpoint(&run.endpoint),
                stats: summarize(&run.records, run.wall_ns),
                latency_samples: latency_samples(&run.records, SAMPLE_LIMIT),
                trajectory: run.trajectory.clone(),
            })
            .collect(),
        maze,
    }
}

pub async fn persist_report(
    dir: PathBuf,
    summary: SummaryReport,
    runs: Vec<TargetRun>,
) -> Result<WrittenPaths> {
    tokio::task::spawn_blocking(move || write_atomic(&dir, summary, &runs))
        .await
        .context("report writer task failed")?
}

fn write_atomic(dir: &Path, summary: SummaryReport, runs: &[TargetRun]) -> Result<WrittenPaths> {
    fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    let stem = format!("{}-{}", summary.scenario, summary.run_id);
    let summary_tmp = dir.join(format!("{stem}.summary.json.tmp"));
    let requests_tmp = dir.join(format!("{stem}.requests.ndjson.tmp"));
    let summary_final = dir.join(format!("{stem}.summary.json"));
    let requests_final = dir.join(format!("{stem}.requests.ndjson"));

    {
        let file = File::create(&summary_tmp)?;
        serde_json::to_writer_pretty(file, &summary)?;
    }
    {
        let file = File::create(&requests_tmp)?;
        let mut writer = BufWriter::new(file);
        for run in runs {
            for rec in &run.records {
                let line = serde_json::json!({
                    "target": run.name,
                    "offset_ns": rec.offset_ns,
                    "latency_ns": rec.latency_ns,
                    "throttle_ns": rec.throttle_ns,
                    "status": rec.status,
                    "action": rec.action,
                    "error": rec.error,
                    "bytes": rec.bytes,
                });
                serde_json::to_writer(&mut writer, &line)?;
                writer.write_all(b"\n")?;
            }
        }
        writer.flush()?;
    }
    replace_file(&summary_tmp, &summary_final)?;
    replace_file(&requests_tmp, &requests_final)?;
    Ok(WrittenPaths {
        summary: summary_final,
        requests: requests_final,
    })
}

fn replace_file(tmp: &Path, dest: &Path) -> Result<()> {
    if dest.exists() {
        fs::remove_file(dest).with_context(|| format!("remove {}", dest.display()))?;
    }
    fs::rename(tmp, dest)
        .with_context(|| format!("rename {} -> {}", tmp.display(), dest.display()))?;
    Ok(())
}

fn redact_endpoint(endpoint: &str) -> String {
    if let Some(idx) = endpoint.find('@') {
        format!("redacted{}", &endpoint[idx..])
    } else {
        endpoint.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::ERR_OK;
    use chrono::Utc;
    use serde_json::json;

    #[test]
    fn schema_and_atomic_write() {
        let dir = std::env::temp_dir().join(format!("jev-report-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let run = TargetRun {
            name: "local".into(),
            model: "jev-quantum-latest".into(),
            endpoint: "http://127.0.0.1:3000/v1/systemone".into(),
            records: vec![RequestRecord {
                offset_ns: 1,
                latency_ns: 12,
                throttle_ns: 0,
                status: 200,
                action: 1,
                error: ERR_OK,
                bytes: 32,
            }],
            wall_ns: 1_000,
            trajectory: None,
        };
        let started = Utc::now();
        let summary = build_summary(
            "latency",
            started,
            json!({"secret": false}),
            std::slice::from_ref(&run),
            None,
        );
        assert_eq!(summary.schema_version, SCHEMA_VERSION);
        let written = write_atomic(&dir, summary, std::slice::from_ref(&run)).unwrap();
        assert!(written.summary.exists());
        assert!(written.requests.exists());
        let parsed: SummaryReport =
            serde_json::from_str(&fs::read_to_string(&written.summary).unwrap()).unwrap();
        assert_eq!(parsed.targets[0].stats.ok, 1);
        let summary2 = build_summary(
            "latency",
            started,
            json!({"secret": false}),
            std::slice::from_ref(&run),
            None,
        );
        write_atomic(&dir, summary2, std::slice::from_ref(&run)).unwrap();
        assert!(written.summary.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn memory_budget_for_million_records() {
        let rec = RequestRecord {
            offset_ns: 0,
            latency_ns: 1,
            throttle_ns: 0,
            status: 200,
            action: 0,
            error: 0,
            bytes: 1,
        };
        let bytes = std::mem::size_of_val(&rec) * 1_000_000;
        assert!(
            bytes < 32 * 1024 * 1024,
            "1M records should stay under 32MiB, got {bytes}"
        );
    }
}
