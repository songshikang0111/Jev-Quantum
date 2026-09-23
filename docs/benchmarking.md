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

Remote maze walks are strictly sequential: a step is retried before its state is advanced. By default, HTTP `429` and `5xx` responses use up to six exponential-backoff retries, starting at one second and capping at 60 seconds. Tune this with `--remote-max-retries` and `--remote-retry-base-ms`; use `--remote-qps` to set the ordinary request pace.

Remote TypeSafe calls are paced at `--remote-qps 10` (set `0` to disable). The wait is stored as `throttle_ns` / `kind: pace_gap` and is not used as animation delay. Local is never paced.

Maze walks stop at the first hard cap: `--max-steps` (per target) or `--max-runtime-secs` (whole run, default 600; `0` disables). Ctrl+C, a time cap, or a finished target writes `summary.json` / `requests.ndjson` immediately. Local measurement does not touch the disk until that target's walk ends.

Remote load requires `--enable-remote-load`. 401/403/429 are classified and never retried.

## What is measured

Reports store **client-observed HTTP latency**. That is not the same as core PRNG time. Compare:

1. `cargo bench -p jev-quantum-core` — in-process decision cost
2. `/metrics` on the server — handler time
3. bench reports — full HTTP round trip

Hot path records stay in preallocated memory. JSON/NDJSON is written on a blocking thread after the measured run.

## Five maze strategies

```bash
cargo run --release -p jev-quantum-bench -- maze-record \
  --targets local,jev,flood-fill,jev-memory,jev-flood-fill --width 10 --height 10 \
  --maze-seed 20260935 --max-steps 10000 --max-runtime-secs 600 \
  --timeout-ms 600000 --warmup 0 --remote-qps 1 \
  --remote-max-retries 8 --remote-retry-base-ms 2000 --output reports/
```

- `local`: existing Quantum random baseline, over HTTP.
- `jev`: existing Jev prompt, over HTTP.
- `flood-fill`: in-process online optimistic BFS distance field, recomputed after observing the current cell's walls. Unknown edges are provisionally open; observed walls are mirrored on both sides. Equal-distance moves prefer unexplored/less visited cells. It never reads walls of remote cells. This follows the [UCI IEEE Micromouse flood-fill approach](https://ieee.ics.uci.edu/micromouse/floodfill.html). It is not an oracle shortest-path run: exploration costs steps.
- `jev-memory`: spatial memory v2. No trajectory is sent or retained by the policy. It sends at most 24 nearby observed cells (coordinates, wall bitmask, visits, directional departure counts), up to four legal moves with lifetime bidirectional edge traversal counts and unexplored-branch counts, plus a stagnation counter. The prompt prefers the least-traversed edge before other exploration tie-breakers. Historical v1 reports with 32 transitions remain readable and are not overwritten.
- `jev-flood-fill`: the same bounded spatial representation plus the optimistic BFS distance at the current cell and at each legal neighbor. The complete observed wall map stays locally in O(width×height) memory; global distances are recalculated locally, and only a local projection goes to Jev. Jev is instructed to select the lowest finite neighbor distance, then unexplored/least-visited ties. All legal options stay in the choice question, and its returned action is applied without a local override. This is Jev choosing from algorithmic features, not Jev computing a whole-maze BFS unaided. Following the field reliably is experimentally measured, not assumed.

Both remote variants use the existing SystemOne `state` + `questions.move` choice schema and ignore `--context minimal`. Request size is bounded by 24 cell rows and four actions rather than the number of walked steps (integer digit counts still grow with coordinates/counters). Tests cover a 100×100 observed map and 100,000 recorded actions under 12 KB per request. No complete trajectory or unobserved wall information is sent. Unlike nearest-cell map truncation alone, the fifth strategy retains the effect of remote known walls through its locally computed global distance field.

The extra targets are maze-only. The seed and positive dimensions are configurable, including 1×N / N×1. Maze generation uses an explicit DFS stack to support larger mazes without recursive stack overflow while retaining the previous seed mapping. Flood-fill needs O(width×height) storage/work per decision; completion still depends on the configured step/time budgets.

`--timeout-ms` controls each HTTP attempt; `--max-runtime-secs` caps the entire invocation including retries and in-flight requests. To give each remote strategy its own ten minutes, run it in a separate invocation. Spatial context is bounded; these are experimental policies, not guarantees of better decisions. The UI renders all imported targets in a responsive grid, with independent colors. In-process planning latency is explicitly distinguished from HTTP latency and should not be interpreted as a network speed comparison.

Merge separately recorded strategies for the demo (checks the entire maze, not just the seed):

```bash
python3 scripts/merge-maze-reports.py \
  reports/0923showcase.local.summary.json reports/0923showcase.jev.summary.json \
  reports/0923showcase-new/maze-*.summary.json \
  --output reports/0923showcase.four-strategies.summary.json \
  --embed web/last-report.js
```

Source run configurations remain in `config.sources`; baseline reports are not overwritten. Reload `/demo/` to load the embedded comparison, or import the combined JSON. Reports and `web/last-report.js` are generated local files ignored by Git.

To inspect a running walk, Ctrl+C persists its trajectory. Continue with
`--resume-report path/to/maze-….summary.json` and the same target, seed, dimensions,
and model; its adjacent `.requests.ndjson` is required. Visits, directional counts,
observed walls, and the recent history are rebuilt from the saved transitions.
Use `--warmup 0` and set `--max-runtime-secs` to the remaining budget: the resume
invocation's deadline starts anew, while its output retains prior decisions and
measurement time. `--max-steps` remains the total decision limit, not extra steps.

For the 0923 five-way comparison, run `jev-flood-fill` and `jev-memory` separately, each with a 600-second budget and the same seed. Progress is printed every 20 decisions, with current coordinates, observed cells and steps since discovery. Each remote decision also records optional `planning` data: request bytes, discovery counts, and for the fifth strategy the current/selected BFS distance and whether the chosen move descends the field. The UI shows peak request size and downhill adherence.

## Luna in one continuing session

`maze-session` is a JSONL game environment for a supervised player. The controller
starts it with a private seed; the player receives only size, exit, coordinates,
current walls and previous-action feedback. Each input is one JSON action, e.g.
`{"action":"RIGHT"}`. The broker counts one decision per input and saves the same
maze-report schema plus `session-transcript.json` and session timing metadata.
It never sends a seed, full map, BFS values or source code to the player.

```bash
cargo run --release -p jev-quantum-bench -- maze-session \
  --width 10 --height 10 --maze-seed 20260935 --max-runtime-secs 600 \
  --output reports/luna-session
```

For cross-agent interaction, the controller can run
`scripts/maze-game-bridge.py` with the same private flags. It binds only to
127.0.0.1:3187 and exposes `POST /move` (one action) and `GET /observation`
(current local observation). The player may only invoke this game interface.
Use a fresh `gpt-5.6-luna` subagent with `fork_turns=none`, never fork the solution
conversation. Keep the same subagent for the entire game and forbid file/source,
process, search and other tool access. This is instruction-based isolation, not
an OS-enforced sandbox. Audit the recorded subagent tool calls afterwards.

`--resume-report` restores the same game after controller interruption; its
adjacent requests file and session transcript are required. Model conversation
memory must remain in the same subagent. It does not restart the player's session.

Timing definitions matter: broker wall time and per-action latency include
controller overhead and parent interaction; these are not inference-only times.
`scripts/audit-maze-session.py` attaches audited model-call count, subagent-turn
count, decision-turn count, active-agent duration, token counts (cached/uncached
separated), and allowed-tool verification from a local rollout. Unavailable token
usage stays null before audit. Cumulative input tokens count re-read context on
each call, not the number of unique prompt tokens.

The final seven-panel report labels historical `jev-memory-v1` separately from
`jev-memory-v2`; `scripts/merge-maze-reports.py --label SOURCE=NAME` can rename a
single-target source for display without modifying the original report.
# Recording the comparison

The demo defaults to one step every 250 ms at 1× (50–100 steps take about
12.5–25 seconds). Choose 0.25× for one step per second, or 0.1× for
2.5 seconds per step. Recorded delays are optional and use the selected multiplier.

Use **Fit all on screen** to select a compact recording layout and automatically
fit all strategies. **View size** adjusts scale from 10% to 150%; **Columns**
selects 1–7 columns. **Show end** displays final paths for screenshots.
**Fullscreen** provides more space. Uncheck **Recording layout** to restore
the full metrics and summary table.

To create a single HTML file that opens directly in Chrome without a server:

```bash
python3 scripts/export-maze-player.py reports/0923showcase.seven-strategies.summary.json web/recording.html
```

The demo's **Offline HTML** link downloads this generated file. Regenerate it
after changing the report or frontend assets. The online demo is served at
`http://127.0.0.1:3001/demo/` when the server runs with `--bind 127.0.0.1:3001`;
loopback URLs must be opened on the machine running the server.

## Pure-code spatial-v2 control

```bash
cargo run --release -p jev-quantum-bench -- maze-record --targets memory-rules \
  --width 10 --height 10 --maze-seed 20260935 --max-steps 10000 \
  --max-runtime-secs 600 --warmup 0 --output reports/memory-rules
python3 scripts/analyze-maze-paths.py reports/0923showcase.eight-strategies.summary.json
```

`memory-rules` uses the same observed-wall and traversal statistics as Jev v2,
with lexicographic priorities: fewest bidirectional edge traversals, unexplored
neighbor, presence of unexplored exits, fewest visits. Complete ties use
UP/RIGHT/DOWN/LEFT, which was unspecified in the model prompt. No model, BFS,
or goal heuristic is used. The path audit separately enumerates all 24 fixed
final-tie direction orders; these are sensitivity checks, not model reruns.
The frontend supports eight strategies and up to eight columns.

## Full-history v1 control

```bash
cargo run --release -p jev-quantum-bench -- maze-record --targets jev-memory-v1-long \
  --width 10 --height 10 --maze-seed 20260935 --max-steps 10000 \
  --max-runtime-secs 1800 --timeout-ms 600000 --warmup 0 --remote-qps 1 \
  --remote-max-retries 8 --remote-retry-base-ms 2000 --output reports/memory-v1-long
```

This restores the original v1 input structure, move instructions and criteria,
changing only the last-32 transition window to the complete chronological history
(`recent_limit: null`). The observed-map limit remains 64; no v2 edge-minimization
rule or BFS feature is added. The new target is separate from existing `jev-memory`
(v2). Run from the start on the same maze with a 30-minute total budget rather than
the historical 10-minute budget. Payload bytes and supplied history length are
recorded per decision. Input size grows with steps; no silent truncation is applied.
A single run with a larger time budget does not isolate the effect of history
length; compare the first 600 seconds separately and report the actual stop reason.

## Full-history v1 prompt-only control

`jev-memory-v1-free` differs from `jev-memory-v1-long` only in
`questions.move.instructions`: replace the Manhattan tie-breaker sentence with
an explicit instruction to choose the strategy judged most effective, without
requiring each move to reduce distance to the exit. All state, chronological
history, map limits, option descriptions, proximity features and the separate
`progress_available` question remain unchanged. This tests the effect of that
instruction, not removal of every goal-proximity cue. A regression test compares
the serialized requests and verifies that only this one field changes.

```bash
cargo run --release -p jev-quantum-bench -- maze-record --targets jev-memory-v1-free \
  --width 10 --height 10 --maze-seed 20260935 --max-steps 10000 \
  --max-runtime-secs 1800 --timeout-ms 600000 --warmup 0 --remote-qps 1 \
  --remote-max-retries 8 --remote-retry-base-ms 2000 --output reports/memory-v1-free
```

## Remove proximity cues while keeping the free-strategy prompt

`jev-memory-no-distance` starts from `jev-memory-v1-free` and removes only the
proximity-guidance group: `state.goal` (derived vector/distance), neighbor
`closer_to_exit`, the proximity sentence in each choice description, and the
`progress_available` question. It keeps the exact same move instructions, exit
coordinates, all transitions, observed-map limit, visit/departure counts and
legal choices. It adds no BFS or least-traversed-edge rule. This is a grouped
feature ablation, not evidence about any one removed field in isolation.

```bash
cargo run --release -p jev-quantum-bench -- maze-record --targets jev-memory-no-distance \
  --width 10 --height 10 --maze-seed 20260935 --max-steps 10000 \
  --max-runtime-secs 1800 --timeout-ms 600000 --warmup 0 --remote-qps 1 \
  --remote-max-retries 8 --remote-retry-base-ms 2000 --output reports/memory-no-distance
```
