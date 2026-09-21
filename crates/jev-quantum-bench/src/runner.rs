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
use crate::pace::{wait_optional, SharedPace, WaitAbort};
use crate::record::{action_from_choice, RequestRecord, ERR_OK};
use crate::report::{build_summary, persist_report, TargetRun};
use crate::target::{http_client, send_request, Target};

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
    let maze = Maze::braided(args.width, args.height, args.maze_seed);
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
    deadline: Instant,
) -> Result<TargetRun> {
    let maze = &job.maze;
    let max_steps = job.max_steps;
    let context = job.context;
    let cancel = &job.cancel;
    let mut visits = vec![0u32; maze.width * maze.height];
    visits[maze.start[1] * maze.width + maze.start[0]] = 1;
    let pace = if target.paced {
        SharedPace::new(remote_qps)
    } else {
        None
    };
    let warmup_req = step_request(
        &target.model,
        maze,
        maze.start[0],
        maze.start[1],
        &visits,
        None,
        context,
    );
    let mut stop = if maze.is_exit(maze.start[0], maze.start[1]) {
        StopReason::Exit
    } else {
        StopReason::MaxSteps
    };
    for _ in 0..warmup {
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
        let _ = send_request(client, target, warmup_req.clone()).await;
    }

    let mut records = Vec::with_capacity(max_steps);
    let mut steps = Vec::new();
    let mut x = maze.start[0];
    let mut y = maze.start[1];
    let mut last_move: Option<String> = None;
    let started = Instant::now();
    let mut success = maze.is_exit(x, y);
    let mut exit_step = None;

    if matches!(stop, StopReason::MaxRuntime | StopReason::Interrupted) && !success {
        return Ok(maze_run(
            target, records, steps, started, success, exit_step, stop,
        ));
    }

    for i in 0..max_steps {
        if success {
            stop = StopReason::Exit;
            break;
        }
        if let Some(reason) = maze_cap_hit(cancel_flag(cancel), Instant::now(), deadline) {
            stop = reason;
            break;
        }
        // Local is never paced: skip the async wait entirely so the hot path is one HTTP call.
        let throttle_ns = if let Some(p) = pace.as_ref() {
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
        let resp = send_request(
            client,
            target,
            step_request(
                &target.model,
                maze,
                x,
                y,
                &visits,
                last_move.as_deref(),
                context,
            ),
        )
        .await;
        let action = resp.body.as_ref().map(decode_move).unwrap_or(255);
        let step = apply_choice(maze, x, y, action, resp.latency_ns, throttle_ns);
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

fn stop_from_wait(abort: WaitAbort) -> StopReason {
    match abort {
        WaitAbort::Cancelled => StopReason::Interrupted,
        WaitAbort::Deadline => StopReason::MaxRuntime,
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
