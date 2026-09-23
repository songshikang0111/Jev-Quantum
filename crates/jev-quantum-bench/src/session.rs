//! JSONL environment broker. The player sees local observations, never the seed/map.
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::time::Instant;

use anyhow::Result;
use chrono::Utc;
use clap::Parser;
use serde_json::{json, Value};

use crate::maze::{apply_choice, Maze, MazeTrajectory, StopReason};
use crate::record::{action_from_choice, RequestRecord, ERR_OK, ERR_SCHEMA};
use crate::report::{build_summary, persist_report, TargetRun};

#[derive(Debug, Clone, Parser)]
pub struct SessionArgs {
    /// Continue the same game after broker/controller interruption.
    #[arg(long)]
    pub resume_report: Option<PathBuf>,
    #[arg(long, default_value_t = 10)]
    pub width: usize,
    #[arg(long, default_value_t = 10)]
    pub height: usize,
    #[arg(long, default_value_t = 7)]
    pub maze_seed: u64,
    #[arg(long, default_value_t = 10_000)]
    pub max_steps: usize,
    #[arg(long, default_value_t = 600)]
    pub max_runtime_secs: u64,
    #[arg(long, default_value = "reports")]
    pub output: PathBuf,
}

fn observation(maze: &Maze, x: usize, y: usize, turns: usize, feedback: Value) -> Value {
    let w = maze.cells[y * maze.width + x];
    json!({
        "event": "observation", "turn": turns, "position": [x,y],
        "size": [maze.width,maze.height], "exit": maze.exit,
        "walls": {"UP":w.n, "RIGHT":w.e, "DOWN":w.s, "LEFT":w.w},
        "feedback": feedback,
        "success": maze.is_exit(x,y),
    })
}

fn emit(value: &Value) -> Result<()> {
    let mut out = std::io::stdout().lock();
    serde_json::to_writer(&mut out, value)?;
    writeln!(out)?;
    out.flush()?;
    Ok(())
}

pub async fn run(args: SessionArgs) -> Result<()> {
    crate::config::require_positive("width", args.width)?;
    crate::config::require_positive("height", args.height)?;
    args.width
        .checked_mul(args.height)
        .ok_or_else(|| anyhow::anyhow!("maze dimensions overflow"))?;
    let maze = Maze::braided(args.width, args.height, args.maze_seed);
    let mut started_at = Utc::now();
    let mut started = Instant::now();
    let deadline = crate::maze::maze_deadline(args.max_runtime_secs);
    let mut x = maze.start[0];
    let mut y = maze.start[1];
    let mut steps = Vec::new();
    let mut records = Vec::new();
    let mut transcript = Vec::new();
    let mut turns = 0;
    let mut stop = StopReason::MaxSteps;
    if let Some(path) = &args.resume_report {
        let prior: crate::report::SummaryReport = serde_json::from_slice(&std::fs::read(path)?)?;
        if prior.maze.as_ref() != Some(&maze) {
            anyhow::bail!("resume maze mismatch");
        }
        let target = prior
            .targets
            .iter()
            .find(|t| t.name == "luna-session")
            .ok_or_else(|| anyhow::anyhow!("no saved session"))?;
        steps = target
            .trajectory
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no trajectory"))?
            .steps
            .clone();
        turns = steps.len();
        if let Some(step) = steps.last() {
            x = step.x;
            y = step.y;
        }
        let records_path = path.with_file_name(
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .replace(".summary.json", ".requests.ndjson"),
        );
        for line in std::fs::read_to_string(records_path)?.lines() {
            records.push(serde_json::from_str(line)?);
        }
        let wall_ns = prior.config["session_metrics"]["wall_ns"]
            .as_u64()
            .unwrap_or(0);
        started -= std::time::Duration::from_nanos(wall_ns);
        started_at = chrono::DateTime::parse_from_rfc3339(&prior.started_at)?.with_timezone(&Utc);
        transcript = serde_json::from_slice(&std::fs::read(
            path.parent().unwrap().join("session-transcript.json"),
        )?)?;
    }
    let initial = observation(
        &maze,
        x,
        y,
        turns,
        steps
            .last()
            .map(|s| json!({"action":s.action,"collision":s.collision,"invalid_action":false}))
            .unwrap_or(Value::Null),
    );
    if transcript.is_empty() {
        transcript.push(json!({"role":"environment","content":initial}));
    }
    emit(&initial)?;
    // Dedicated thread instead of Tokio's blocking stdin: a deadline can exit without
    // waiting for an unfinished stdin read during runtime shutdown.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            match line {
                Ok(line) => {
                    if tx.send(line).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    while turns < args.max_steps && !maze.is_exit(x, y) {
        let waiting = Instant::now();
        let line = tokio::select! {
            line = rx.recv() => match line { Some(line) => line, None => { stop=StopReason::Interrupted; break; } },
            _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => { stop=StopReason::MaxRuntime; break; },
            _ = tokio::signal::ctrl_c() => { stop=StopReason::Interrupted; break; },
        };
        let latency_ns = waiting.elapsed().as_nanos() as u64;
        let payload: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
        if payload["command"] == "stop" {
            stop = StopReason::Interrupted;
            break;
        }
        let action = payload["action"]
            .as_str()
            .map(action_from_choice)
            .unwrap_or(255);
        turns += 1;
        transcript.push(
            json!({"role":"player","turn":turns,"content":line,"response_wait_ns":latency_ns}),
        );
        let step = apply_choice(&maze, x, y, action, latency_ns, 0);
        x = step.x;
        y = step.y;
        records.push(RequestRecord {
            offset_ns: waiting.duration_since(started).as_nanos() as u64,
            latency_ns,
            throttle_ns: 0,
            status: if action < 4 { 200 } else { 422 },
            action,
            error: if action < 4 { ERR_OK } else { ERR_SCHEMA },
            bytes: line.len() as u32,
        });
        let feedback = json!({"action":step.action,"collision":step.collision,
            "invalid_action":action>=4});
        steps.push(step);
        let obs = observation(&maze, x, y, turns, feedback);
        transcript.push(json!({"role":"environment","content":obs}));
        emit(&obs)?;
    }
    let success = maze.is_exit(x, y);
    if success {
        stop = StopReason::Exit;
    }
    let wall_ns = started.elapsed().as_nanos() as u64;
    let run = TargetRun {
        name: "luna-session".into(),
        model: "gpt-5.6-luna".into(),
        endpoint: "subagent-session".into(),
        records,
        wall_ns,
        trajectory: Some(MazeTrajectory {
            steps,
            success,
            exit_step: success.then_some(turns),
            stop_reason: stop,
        }),
    };
    let config = json!({"targets":"luna-session", "max_steps":args.max_steps,
        "max_runtime_secs":args.max_runtime_secs, "api_key":"not-used",
        "session_metrics":{"model_turns":turns,"environment_observations":turns+1,"wall_ns":wall_ns,
            "token_usage":null,"latency_definition":"environment emission to submitted action, includes model and orchestration; not pure inference latency",
            "context":"single continuing subagent session; only local walls, coordinates, size, exit and action feedback",
            "isolation":"no inherited task history; only the game interface is allowed; no file/source access; broker omits seed and full map. Instruction-based isolation, not an OS sandbox."}});
    let summary = build_summary("maze", started_at, config, &[run.clone()], Some(maze));
    let paths = persist_report(args.output.clone(), summary, vec![run]).await?;
    let transcript_path = args.output.join("session-transcript.json");
    std::fs::write(&transcript_path, serde_json::to_vec_pretty(&transcript)?)?;
    emit(
        &json!({"event":"finished","turns":turns,"success":success,"stop_reason":stop.as_str(),"wall_ns":wall_ns}),
    )?;
    eprintln!(
        "session report: {}; transcript: {}",
        paths.summary.display(),
        transcript_path.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn player_observation_never_exposes_seed_or_global_map() {
        let maze = Maze::braided(10, 10, 20260935);
        let obs = observation(&maze, 0, 0, 0, Value::Null);
        let object = obs.as_object().unwrap();
        assert_eq!(object.len(), 8);
        assert!(!object.contains_key("seed"));
        assert!(!object.contains_key("cells"));
        assert_eq!(obs["walls"].as_object().unwrap().len(), 4);
        assert_eq!(obs["walls"]["RIGHT"], maze.cells[0].e);
        assert!(!serde_json::to_string(&obs).unwrap().contains("20260935"));
    }
}
