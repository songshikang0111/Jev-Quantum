use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use chrono::Utc;
use indexmap::IndexMap;
use jev_quantum_core::protocol::{Question, SystemOneRequest};
use serde_json::json;
use tokio::sync::Semaphore;

use crate::config::{load_merged, LoadArgs, MazeArgs, RunArgs, TargetKind};
use crate::maze::{
    apply_choice, cancel_flag, decode_move, maze_cap_hit, maze_deadline, pace_gap_step,
    step_request, Maze, MazeContext, StopReason,
};
use crate::navigation::Navigation;
use crate::pace::{wait_optional, SharedPace, WaitAbort};
use crate::record::{action_from_choice, RequestRecord, ERR_OK};
use crate::report::{build_summary, persist_report, TargetRun};
use crate::target::{http_client, send_request, Target, TargetResponse};

pub async fn run_latency(args: RunArgs) -> Result<()> {
    execute("latency", args, false, None).await
}

pub async fn run_load(args: LoadArgs) -> Result<()> {
    if args.run.targets.to_ascii_lowercase().contains("jev") && !args.enable_remote_load {
        bail!("remote load against TypeSafe Jev requires --enable-remote-load");
    }
    if args.run.targets.to_ascii_lowercase().contains("jev") && args.enable_remote_load {
        eprintln!(
            "WARNING: sending {} requests at concurrency {} to TypeSafe Jev. This may incur cost.",
            args.run.requests, args.run.concurrency
        );
    }
    execute("load", args.run, args.enable_remote_load, None).await
}

pub async fn run_maze(args: MazeArgs) -> Result<()> {
    crate::config::require_positive("width", args.width)?;
    crate::config::require_positive("height", args.height)?;
    args.width
        .checked_mul(args.height)
        .ok_or_else(|| anyhow::anyhow!("maze dimensions overflow"))?;
    let maze = Maze::braided(args.width, args.height, args.maze_seed);
    let resume = if let Some(path) = &args.resume_report {
        let report: crate::report::SummaryReport = serde_json::from_slice(&std::fs::read(path)?)?;
        if report.maze.as_ref() != Some(&maze) {
            bail!("resume report maze does not match dimensions/seed");
        }
        let records_path = path.with_file_name(
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .replace(".summary.json", ".requests.ndjson"),
        );
        let records = std::fs::read_to_string(records_path)?;
        Some((report, records))
    } else {
        None
    };
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let cancel = Arc::clone(&cancel);
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                cancel.store(true, Ordering::Relaxed);
                eprintln!("interrupt received; persisting maze log then exiting");
            }
        });
    }
    execute(
        "maze",
        args.run,
        false,
        Some(MazeJob {
            maze,
            resume,
            max_steps: args.max_steps,
            max_runtime_secs: args.max_runtime_secs,
            context: args.context,
            cancel,
        }),
    )
    .await
}

struct MazeJob {
    maze: Maze,
    resume: Option<(crate::report::SummaryReport, String)>,
    max_steps: usize,
    max_runtime_secs: u64,
    context: MazeContext,
    cancel: Arc<AtomicBool>,
}

async fn execute(
    scenario: &str,
    args: RunArgs,
    _remote_load: bool,
    maze: Option<MazeJob>,
) -> Result<()> {
    crate::config::require_positive("requests", args.requests)?;
    crate::config::require_positive("concurrency", args.concurrency)?;
    let config = load_merged(&args)?;
    println!("{}", config.describe());

    let wanted = TargetKind::parse_list(&args.targets)?;
    if maze.is_none()
        && wanted.iter().any(|k| {
            matches!(
                k,
                TargetKind::FloodFill | TargetKind::JevMemory | TargetKind::JevFloodFill
            )
        })
    {
        bail!("flood-fill, jev-memory and jev-flood-fill are maze-record strategies only");
    }
    let mut targets = Vec::new();
    for kind in wanted {
        match Target::from_config(kind, &config) {
            Some(target) => targets.push(target),
            None => eprintln!(
                "skipping {} (AI_GATEWAY_API_KEY missing); local tests still run",
                kind.as_str()
            ),
        }
    }
    if targets.is_empty() {
        bail!("no usable targets");
    }

    let timeout = Duration::from_millis(args.timeout_ms);
    let client = http_client(timeout)?;
    let mut runs = Vec::new();
    let maze_ref = maze.as_ref().map(|job| job.maze.clone());
    let context_label = maze.as_ref().map(|job| job.context.as_str());
    let started_at = Utc::now();
    let report_config = json!({
        "targets": args.targets,
        "requests": args.requests,
        "concurrency": args.concurrency,
        "warmup": args.warmup,
        "maze_context": context_label,
        "remote_qps": args.remote_qps,
        "timeout_ms": args.timeout_ms,
        "resumed_from": maze.as_ref().and_then(|job| job.resume.as_ref().map(|(report, _)| &report.run_id)),
        "strategy_contexts": {"flood-fill": "online optimistic BFS with observed walls", "jev-memory": "spatial-v2: 24 observed cells + lifetime edge traversal counts + stagnation", "jev-flood-fill": "flood-v1: spatial memory + locally computed optimistic BFS gradient; unmodified Jev choice"},
        "remote_max_retries": args.remote_max_retries,
        "remote_retry_base_ms": args.remote_retry_base_ms,
        "max_steps": maze.as_ref().map(|job| job.max_steps),
        "max_runtime_secs": maze.as_ref().map(|job| job.max_runtime_secs),
        "local_base_url": config.local_base_url,
        "gateway_base_url": config.gateway_base_url,
        "local_model": config.local_model,
        "gateway_model": config.gateway_model,
        "api_key": "redacted",
    });
    let out_dir = config.report_output_dir.clone();
    let persist = |runs: Vec<TargetRun>, maze_copy: Option<Maze>| {
        let summary = build_summary(
            scenario,
            started_at,
            report_config.clone(),
            &runs,
            maze_copy,
        );
        persist_report(out_dir.clone(), summary, runs)
    };

    let deadline = maze.as_ref().map(|job| maze_deadline(job.max_runtime_secs));

    for (idx, target) in targets.iter().enumerate() {
        if let Some(job) = &maze {
            if let Some(reason) = maze_cap_hit(
                cancel_flag(&job.cancel),
                Instant::now(),
                deadline.expect("maze deadline"),
            ) {
                eprintln!("stopping before {}: {}", target.name, reason.as_str());
                break;
            }
            let run = run_maze_target(
                &client,
                target,
                job,
                args.warmup,
                args.remote_qps,
                args.remote_max_retries,
                args.remote_retry_base_ms,
                deadline.expect("maze deadline"),
            )
            .await?;
            let stop = run
                .trajectory
                .as_ref()
                .map(|t| t.stop_reason)
                .unwrap_or(StopReason::Unspecified);
            print_stats(&run);
            runs.push(run);
            let paths = persist(runs.clone(), maze_ref.clone()).await?;
            println!("checkpoint {}", paths.summary.display());
            if stop.aborts_remaining_targets() {
                if idx + 1 < targets.len() {
                    eprintln!(
                        "hard cap reached ({}); remaining targets skipped",
                        stop.as_str()
                    );
                }
                break;
            }
        } else {
            let run = run_fixed_target(&client, target, &args).await?;
            print_stats(&run);
            runs.push(run);
        }
    }

    let paths = persist(runs, maze_ref).await?;
    println!("wrote {}", paths.summary.display());
    println!("wrote {}", paths.requests.display());
    Ok(())
}

async fn run_fixed_target(
    client: &reqwest::Client,
    target: &Target,
    args: &RunArgs,
) -> Result<TargetRun> {
    let payload = sample_payload(&target.model);
    let pace = if target.paced {
        SharedPace::new(args.remote_qps)
    } else {
        None
    };
    for _ in 0..args.warmup {
        let _ = wait_optional(pace.as_ref()).await;
        let _ = send_request(client, target, payload.clone()).await;
    }

    let mut records = Vec::with_capacity(args.requests);
    records.resize(
        args.requests,
        RequestRecord {
            offset_ns: 0,
            latency_ns: 0,
            throttle_ns: 0,
            status: 0,
            action: 255,
            error: ERR_OK,
            bytes: 0,
        },
    );
    let next = Arc::new(AtomicUsize::new(0));
    let started = Instant::now();
    let sem = Arc::new(Semaphore::new(args.concurrency.max(1)));
    let mut joins = Vec::new();
    for _ in 0..args.concurrency.max(1) {
        let sem = Arc::clone(&sem);
        let next = Arc::clone(&next);
        let client = client.clone();
        let target = target.clone();
        let payload = payload.clone();
        let pace = pace.clone();
        let total = args.requests;
        joins.push(tokio::spawn(async move {
            let mut local = Vec::new();
            loop {
                let idx = next.fetch_add(1, Ordering::Relaxed);
                if idx >= total {
                    break;
                }
                let _permit = sem.acquire().await.expect("semaphore");
                let throttle_ns = wait_optional(pace.as_ref()).await;
                let offset = started.elapsed().as_nanos() as u64;
                let resp = send_request(&client, &target, payload.clone()).await;
                drop(_permit);
                let action = resp
                    .body
                    .as_ref()
                    .and_then(|b| b.answers.values().next())
                    .and_then(|a| a.choice_label())
                    .map(action_from_choice)
                    .unwrap_or(255);
                local.push((
                    idx,
                    RequestRecord {
                        offset_ns: offset,
                        latency_ns: resp.latency_ns,
                        throttle_ns,
                        status: resp.status,
                        action,
                        error: resp.error,
                        bytes: resp.bytes,
                    },
                ));
            }
            local
        }));
    }
    for join in joins {
        for (idx, rec) in join.await? {
            records[idx] = rec;
        }
    }
    Ok(TargetRun {
        name: target.name.clone(),
        model: target.model.clone(),
        endpoint: target.endpoint.clone(),
        records,
        wall_ns: started.elapsed().as_nanos() as u64,
        trajectory: None,
    })
}

async fn run_maze_target(
    client: &reqwest::Client,
    target: &Target,
    job: &MazeJob,
    warmup: usize,
    remote_qps: f64,
    remote_max_retries: u32,
    remote_retry_base_ms: u64,
    deadline: Instant,
) -> Result<TargetRun> {
    let maze = &job.maze;
    let max_steps = job.max_steps;
    let context = if matches!(target.name.as_str(), "jev-memory" | "jev-flood-fill") {
        MazeContext::Features
    } else {
        job.context
    };
    let mut navigation = Navigation::new(maze.width, maze.height);
    let cancel = &job.cancel;
    let mut visits = vec![0u32; maze.width * maze.height];
    visits[maze.start[1] * maze.width + maze.start[0]] = 1;
    let pace = if target.paced {
        SharedPace::new(remote_qps)
    } else {
        None
    };
    let mut warmup_req = step_request(
        &target.model,
        maze,
        maze.start[0],
        maze.start[1],
        &visits,
        None,
        context,
    );
    if matches!(target.name.as_str(), "jev-memory" | "jev-flood-fill") {
        let mut warmup_navigation = Navigation::new(maze.width, maze.height);
        warmup_navigation.observe(
            maze.start[0],
            maze.start[1],
            maze.cells[maze.start[1] * maze.width + maze.start[0]],
        );
        warmup_navigation.enrich(
            &mut warmup_req,
            maze.start[0],
            maze.start[1],
            maze.exit,
            target.name == "jev-flood-fill",
        );
    }
    let mut stop = if maze.is_exit(maze.start[0], maze.start[1]) {
        StopReason::Exit
    } else {
        StopReason::MaxSteps
    };
    for _ in 0..if target.name == "flood-fill" {
        0
    } else {
        warmup
    } {
        if let Some(reason) = maze_cap_hit(cancel_flag(cancel), Instant::now(), deadline) {
            stop = reason;
            break;
        }
        if let Some(p) = pace.as_ref() {
            if let Err(abort) = p.wait_until(cancel, deadline).await {
                stop = stop_from_wait(abort);
                break;
            }
        }
        if let Err(abort) =
            send_maze_request(client, target, warmup_req.clone(), cancel, deadline).await
        {
            stop = stop_from_wait(abort);
            break;
        }
    }

    let mut records = Vec::with_capacity(max_steps);
    let mut steps = Vec::new();
    let mut x = maze.start[0];
    let mut y = maze.start[1];
    let mut last_move: Option<String> = None;
    let mut started = Instant::now();
    let mut completed = 0;
    if let Some((report, ndjson)) = &job.resume {
        let prior = report
            .targets
            .iter()
            .find(|t| t.name == target.name)
            .ok_or_else(|| anyhow::anyhow!("resume report has no {}", target.name))?;
        if prior.model != target.model {
            bail!("resume model mismatch");
        }
        let version = match target.name.as_str() {
            "jev-memory" => Some("spatial-v2:"),
            "jev-flood-fill" => Some("flood-v1:"),
            _ => None,
        };
        if let Some(version) = version {
            let saved = report.config["strategy_contexts"][&target.name]
                .as_str()
                .unwrap_or("");
            if !saved.starts_with(version) {
                bail!(
                    "resume strategy version mismatch for {}; start a fresh comparison run",
                    target.name
                );
            }
        }
        let trajectory = prior
            .trajectory
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no saved trajectory"))?;
        for step in &trajectory.steps {
            if step.kind == crate::maze::StepKind::PaceGap {
                continue;
            }
            navigation.observe(x, y, maze.cells[y * maze.width + x]);
            navigation.record(
                [x, y],
                [step.x, step.y],
                action_from_choice(&step.action),
                step.collision,
            );
            x = step.x;
            y = step.y;
            visits[y * maze.width + x] = visits[y * maze.width + x].saturating_add(1);
            last_move = Some(step.action.clone());
            completed += 1;
        }
        steps = trajectory.steps.clone();
        for line in ndjson.lines() {
            let value: serde_json::Value = serde_json::from_str(line)?;
            if value["target"].as_str() == Some(&target.name) {
                records.push(serde_json::from_value(value)?);
            }
        }
        if prior.stats.qps > 0.0 {
            started -= Duration::from_secs_f64(prior.stats.requests as f64 / prior.stats.qps);
        }
        eprintln!(
            "resuming {} after {} decisions at ({},{})",
            target.name, completed, x, y
        );
    }
    let mut success = maze.is_exit(x, y);
    let mut exit_step = success.then_some(completed);

    if matches!(stop, StopReason::MaxRuntime | StopReason::Interrupted) && !success {
        return Ok(maze_run(
            target, records, steps, started, success, exit_step, stop,
        ));
    }

    for i in completed..max_steps {
        if success {
            stop = StopReason::Exit;
            break;
        }
        if let Some(reason) = maze_cap_hit(cancel_flag(cancel), Instant::now(), deadline) {
            stop = reason;
            break;
        }
        // Local is never paced: skip the async wait entirely so the hot path is one HTTP call.
        let mut throttle_ns = if let Some(p) = pace.as_ref() {
            match p.wait_until(cancel, deadline).await {
                Ok(wait) => wait.as_nanos() as u64,
                Err(abort) => {
                    stop = stop_from_wait(abort);
                    break;
                }
            }
        } else {
            0
        };
        if throttle_ns > 0 {
            steps.push(pace_gap_step(x, y, throttle_ns));
        }
        let offset = started.elapsed().as_nanos() as u64;
        let decision_started = Instant::now();
        if matches!(
            target.name.as_str(),
            "flood-fill" | "jev-memory" | "jev-flood-fill"
        ) {
            navigation.observe(x, y, maze.cells[y * maze.width + x]);
        }
        let mut request = step_request(
            &target.model,
            maze,
            x,
            y,
            &visits,
            last_move.as_deref(),
            context,
        );
        if matches!(target.name.as_str(), "jev-memory" | "jev-flood-fill") {
            navigation.enrich(
                &mut request,
                x,
                y,
                maze.exit,
                target.name == "jev-flood-fill",
            );
        }
        let planned = if target.name == "flood-fill" {
            navigation.choose(x, y, maze.exit)
        } else {
            None
        };
        let mut resp = if target.name == "flood-fill" {
            TargetResponse {
                status: 200,
                latency_ns: decision_started.elapsed().as_nanos() as u64,
                bytes: 0,
                body: None,
                error: ERR_OK,
            }
        } else {
            match send_maze_request(client, target, request.clone(), cancel, deadline).await {
                Ok(resp) => resp,
                Err(abort) => {
                    stop = stop_from_wait(abort);
                    break;
                }
            }
        };
        let mut retries = 0;
        while target.paced
            && is_retryable_remote_status(resp.status)
            && retries < remote_max_retries
        {
            retries += 1;
            let delay = remote_retry_delay(remote_retry_base_ms, retries);
            eprintln!(
                "{} step {} returned {}; retry {}/{} in {}ms",
                target.name,
                i + 1,
                resp.status,
                retries,
                remote_max_retries,
                delay.as_millis()
            );
            if let Err(abort) = wait_for_retry(cancel, deadline, delay).await {
                stop = stop_from_wait(abort);
                break;
            }
            if let Some(p) = pace.as_ref() {
                match p.wait_until(cancel, deadline).await {
                    Ok(wait) => throttle_ns = throttle_ns.saturating_add(wait.as_nanos() as u64),
                    Err(abort) => {
                        stop = stop_from_wait(abort);
                        break;
                    }
                }
            }
            resp = match send_maze_request(client, target, request.clone(), cancel, deadline).await
            {
                Ok(resp) => resp,
                Err(abort) => {
                    stop = stop_from_wait(abort);
                    break;
                }
            };
        }
        if !matches!(stop, StopReason::MaxSteps) {
            break;
        }
        if resp.error != ERR_OK {
            records.push(RequestRecord {
                offset_ns: offset,
                latency_ns: resp.latency_ns,
                throttle_ns,
                status: resp.status,
                action: 255,
                error: resp.error,
                bytes: resp.bytes,
            });
            stop = StopReason::RequestFailed;
            break;
        }
        let action = planned
            .or_else(|| resp.body.as_ref().map(decode_move))
            .unwrap_or(255);
        let mut step = apply_choice(maze, x, y, action, resp.latency_ns, throttle_ns);
        if matches!(target.name.as_str(), "jev-memory" | "jev-flood-fill") {
            step.planning = Some(json!({
                "request_bytes": serde_json::to_vec(&request)?.len(),
                "observed_cells": navigation.observed_count(),
                "steps_without_discovery": navigation.stagnant_steps(),
                "distance_here": request.state["flood_fill"]["distance_here"],
                "selected_distance": request.state["moves"][crate::record::action_name(action)]["distance_to_exit"],
                "downhill": request.state["moves"][crate::record::action_name(action)]["downhill"],
            }));
            if (i + 1) % 20 == 0 {
                eprintln!(
                    "{} progress: {} steps, position=({},{}), explored={}, stagnant={}",
                    target.name,
                    i + 1,
                    step.x,
                    step.y,
                    navigation.observed_count(),
                    navigation.stagnant_steps()
                );
            }
        }
        if matches!(
            target.name.as_str(),
            "flood-fill" | "jev-memory" | "jev-flood-fill"
        ) {
            navigation.record([x, y], [step.x, step.y], action, step.collision);
        }
        x = step.x;
        y = step.y;
        visits[y * maze.width + x] = visits[y * maze.width + x].saturating_add(1);
        last_move = Some(step.action.clone());
        if maze.is_exit(x, y) {
            success = true;
            exit_step = Some(i + 1);
            stop = StopReason::Exit;
        }
        records.push(RequestRecord {
            offset_ns: offset,
            latency_ns: resp.latency_ns,
            throttle_ns,
            status: resp.status,
            action,
            error: resp.error,
            bytes: resp.bytes,
        });
        steps.push(step);
        if success {
            break;
        }
        if let Some(reason) = maze_cap_hit(cancel_flag(cancel), Instant::now(), deadline) {
            stop = reason;
            break;
        }
    }

    Ok(maze_run(
        target, records, steps, started, success, exit_step, stop,
    ))
}

async fn send_maze_request(
    client: &reqwest::Client,
    target: &Target,
    request: SystemOneRequest,
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<TargetResponse, WaitAbort> {
    tokio::select! {
        resp = send_request(client, target, request) => Ok(resp),
        _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => Err(WaitAbort::Deadline),
        _ = async { while !cancel_flag(cancel) { tokio::time::sleep(Duration::from_millis(50)).await; } } => Err(WaitAbort::Cancelled),
    }
}

fn is_retryable_remote_status(status: u16) -> bool {
    status == 429 || (500..600).contains(&status)
}

fn remote_retry_delay(base_ms: u64, retry: u32) -> Duration {
    let multiplier = 1u64 << retry.saturating_sub(1).min(6);
    Duration::from_millis(base_ms.saturating_mul(multiplier).min(60_000))
}

async fn wait_for_retry(
    cancel: &AtomicBool,
    deadline: Instant,
    delay: Duration,
) -> Result<(), WaitAbort> {
    let until = Instant::now() + delay;
    loop {
        if cancel_flag(cancel) {
            return Err(WaitAbort::Cancelled);
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(WaitAbort::Deadline);
        }
        let remaining = until.saturating_duration_since(now);
        if remaining.is_zero() {
            return Ok(());
        }
        tokio::time::sleep(
            remaining
                .min(Duration::from_millis(50))
                .min(deadline.saturating_duration_since(now)),
        )
        .await;
    }
}

fn stop_from_wait(abort: WaitAbort) -> StopReason {
    match abort {
        WaitAbort::Cancelled => StopReason::Interrupted,
        WaitAbort::Deadline => StopReason::MaxRuntime,
    }
}

#[cfg(test)]
mod tests {
    use super::remote_retry_delay;
    use std::time::Duration;

    #[tokio::test]
    async fn deadline_interrupts_in_flight_http() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(10)).await;
        });
        let target = super::Target {
            name: "jev-memory".into(),
            endpoint: format!("http://{addr}"),
            model: "test".into(),
            api_key: None,
            paced: true,
        };
        let client = super::http_client(Duration::from_secs(10)).unwrap();
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let started = std::time::Instant::now();
        let result = super::send_maze_request(
            &client,
            &target,
            super::sample_payload("test"),
            &cancel,
            started + Duration::from_millis(30),
        )
        .await;
        assert!(matches!(result, Err(crate::pace::WaitAbort::Deadline)));
        assert!(started.elapsed() < Duration::from_secs(1));
        server.abort();
    }

    #[test]
    fn remote_retries_back_off_and_cap() {
        assert_eq!(remote_retry_delay(1_000, 1), Duration::from_secs(1));
        assert_eq!(remote_retry_delay(1_000, 4), Duration::from_secs(8));
        assert_eq!(remote_retry_delay(20_000, 6), Duration::from_secs(60));
    }
}

fn maze_run(
    target: &Target,
    records: Vec<RequestRecord>,
    steps: Vec<crate::maze::MazeStep>,
    started: Instant,
    success: bool,
    exit_step: Option<usize>,
    stop: StopReason,
) -> TargetRun {
    TargetRun {
        name: target.name.clone(),
        model: target.model.clone(),
        endpoint: target.endpoint.clone(),
        records,
        wall_ns: started.elapsed().as_nanos() as u64,
        trajectory: Some(crate::maze::MazeTrajectory {
            steps,
            success,
            exit_step,
            stop_reason: stop,
        }),
    }
}

fn sample_payload(model: &str) -> SystemOneRequest {
    let mut criteria = IndexMap::new();
    criteria.insert("billing".into(), Some("Payments".into()));
    criteria.insert("technical".into(), Some("Bugs".into()));
    criteria.insert("sales".into(), Some("Pricing".into()));
    let mut questions = IndexMap::new();
    questions.insert(
        "queue".into(),
        Question::Choice {
            instructions: Some("Which team should handle this?".into()),
            criteria,
        },
    );
    questions.insert(
        "urgent".into(),
        Question::Noul {
            instructions: Some("Is this urgent?".into()),
            criteria: None,
        },
    );
    SystemOneRequest {
        model: model.to_string(),
        state: json!("Customer was charged twice and wants a refund today."),
        questions,
    }
}

fn print_stats(run: &TargetRun) {
    let stats = crate::stats::summarize(&run.records, run.wall_ns);
    match run.trajectory.as_ref() {
        Some(traj) => println!(
            "{:<8} ok={}/{} qps={:.1} p50={:.3}ms p95={:.3}ms p99={:.3}ms max={:.3}ms stop={}",
            run.name,
            stats.ok,
            stats.requests,
            stats.qps,
            stats.p50_ns as f64 / 1e6,
            stats.p95_ns as f64 / 1e6,
            stats.p99_ns as f64 / 1e6,
            stats.max_ns as f64 / 1e6,
            traj.stop_reason.as_str()
        ),
        None => println!(
            "{:<8} ok={}/{} qps={:.1} p50={:.3}ms p95={:.3}ms p99={:.3}ms max={:.3}ms",
            run.name,
            stats.ok,
            stats.requests,
            stats.qps,
            stats.p50_ns as f64 / 1e6,
            stats.p95_ns as f64 / 1e6,
            stats.p99_ns as f64 / 1e6,
            stats.max_ns as f64 / 1e6
        ),
    }
}
