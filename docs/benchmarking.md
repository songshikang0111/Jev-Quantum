# Benchmarking

`jev-quantum-bench` compares the local server with Jev through the **TypeSafe official API**.

## Configure

```bash
cp .env.example .env
```

Fill `AI_GATEWAY_API_KEY`. Do not pass the key on the command line.

| Variable | Default |
| --- | --- |
| `LOCAL_API_BASE_URL` | `http://127.0.0.1:3000` |
| `LOCAL_API_MODEL` | `jev-quantum-latest` |
| `AI_GATEWAY_BASE_URL` | `https://api.typesafe.ai` |
| `AI_GATEWAY_MODEL` | `jev-latest` |
| `REPORT_OUTPUT_DIR` | `reports` |

Remote calls use `POST {AI_GATEWAY_BASE_URL}/v1/systemone` with `Authorization: Bearer $AI_GATEWAY_API_KEY`. Default target is `https://api.typesafe.ai/v1/systemone` and model `jev-latest`.

## Commands

```bash
cargo run --release -p jev-quantum-bench -- latency --targets local,jev --requests 100
cargo run --release -p jev-quantum-bench -- load --targets local --requests 100000 --concurrency 64
cargo run --release -p jev-quantum-bench -- load --targets local,jev --requests 200 --concurrency 4 --enable-remote-load
cargo run --release -p jev-quantum-bench -- maze-record --targets local,jev --width 10 --height 10 --max-steps 200 --max-runtime-secs 600 --output reports/
```

Maze requests default to `--context features`: legal moves only, plus goal vector, visit counts, neighbor flags, and a `progress_available` noul. Both local Quantum and TypeSafe Jev receive the same payload. Use `--context minimal` for the old four-direction bare request.

Missing `AI_GATEWAY_API_KEY` skips the `jev` target and still runs local measurements.

Remote TypeSafe calls are paced at `--remote-qps 10` (set `0` to disable). The wait is stored as `throttle_ns` / `kind: pace_gap` and is not used as animation delay. Local is never paced.

Maze walks stop at the first hard cap: `--max-steps` (per target) or `--max-runtime-secs` (whole run, default 600; `0` disables). Ctrl+C, a time cap, or a finished target writes `summary.json` / `requests.ndjson` immediately. Local measurement does not touch the disk until that target's walk ends.

Remote load requires `--enable-remote-load`. 401/403/429 are classified and never retried.

## What is measured

Reports store **client-observed HTTP latency**. That is not the same as core PRNG time. Compare:

1. `cargo bench -p jev-quantum-core` — in-process decision cost
2. `/metrics` on the server — handler time
3. bench reports — full HTTP round trip

Hot path records stay in preallocated memory. JSON/NDJSON is written on a blocking thread after the measured run.
