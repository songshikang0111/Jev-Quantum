# Published experiment results

These reviewed artifacts contain maze definitions, action trajectories, aggregate
statistics and client-observed request timing. `api_key` entries are placeholders,
not credentials. Environment files, runtime logs, PID files and raw agent sessions
remain ignored.

- `article-selected-seven.summary.json`: seven article groups with display names.
- `article-selected-seven.player.html`: self-contained article replay (up to 64×).
- `0923showcase.eleven-strategies.summary.json`: full eleven-version comparison.
- `0923showcase.player.html`: full comparison replay.
- `memory-no-distance-fork/`: separate checkpoint continuation experiment. The first
  78 decisions are inherited from the old policy; the following 38 use the new one.
  Checkpoint configuration records the source and policy switch explicitly.
- Subdirectory `.summary.json` / `.requests.ndjson` pairs retain recorded sources.

Timing definitions and experimental limitations are described in
[`../docs/maze-strategies.md`](../docs/maze-strategies.md) and
[`../docs/benchmarking.md`](../docs/benchmarking.md).

Regenerate article replay with:

```bash
python3 scripts/export-maze-player.py reports/article-selected-seven.summary.json reports/article-selected-seven.player.html
```
