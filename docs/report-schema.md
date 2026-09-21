# Report schema

`schema_version` is `1`.

## `*.summary.json`

Used by the offline maze page.

```json
{
  "schema_version": 1,
  "run_id": "20260921T010203",
  "scenario": "latency | load | maze",
  "started_at": "RFC3339",
  "finished_at": "RFC3339",
  "config": { "api_key": "redacted" },
  "targets": [
    {
      "name": "local",
      "model": "jev-quantum-latest",
      "endpoint": "http://127.0.0.1:3000/v1/systemone",
      "stats": {
        "requests": 100,
        "ok": 100,
        "qps": 12345.6,
        "p50_ns": 1,
        "p95_ns": 2,
        "p99_ns": 3,
        "max_ns": 4,
        "mean_ns": 5,
        "errors": {}
      },
      "latency_samples": [1, 2, 3],
      "trajectory": {
        "steps": [
          {"x": 0, "y": 0, "action": "RIGHT", "collision": false, "latency_ns": 1000, "throttle_ns": 0, "kind": "decision"}
        ],
        "success": true,
        "exit_step": 12,
        "stop_reason": "exit"
      }
    }
  ],
  "maze": {
    "width": 15,
    "height": 15,
    "seed": 7,
    "start": [0, 0],
    "exit": [14, 14],
    "cells": [{"n": true, "e": false, "s": true, "w": true}]
  }
}
```

`stop_reason` is `exit`, `max_steps`, `max_runtime`, `interrupted`, or `unspecified` (missing in older reports). Maze reports are rewritten after each target and again on Ctrl+C / cap so a partial remote walk still keeps the local trajectory.

## `*.requests.ndjson`

One compact record per request. Written after measurement using a temporary file + replace, so Windows can overwrite a checkpoint.
