# Maze exit timing report

Single-run measurements against the local Jev Quantum HTTP API.
Each size uses a braided maze, seed `7`, start `[0,0]`, exit `[n-1,n-1]`.
Every step is one sequential `POST /v1/systemone` choice among `UP/RIGHT/DOWN/LEFT`.
Wall-clock below is the measured request loop (`requests / qps`), not report serialization.

| Size | Cells | Exit found | Steps | Wall clock | QPS | p50 | Report |
| --- | ---: | --- | ---: | ---: | ---: | ---: | --- |
| 10×10 | 100 | yes | 13,328 | 2.06 s | 6,456 | 0.142 ms | `reports/maze-20260921T020159.summary.json` |
| 20×20 | 400 | yes | 7,653 | 1.21 s | 6,328 | 0.141 ms | `reports/maze-20260921T020648.summary.json` |
| 30×30 | 900 | yes | 79,067 | 14.90 s | 5,307 | 0.169 ms | `reports/maze-20260921T020704.summary.json` |
| 40×40 | 1,600 | yes | 176,626 | 31.65 s | 5,581 | 0.161 ms | `reports/maze-20260921T020751.summary.json` |
| 100×100 | 10,000 | yes | 273,217 | 50.91 s | 5,367 | 0.166 ms | `reports/maze-20260921T020358.summary.json` |

## Notes

- These are **one seed, one walk each**. Random-walk hitting time has high variance; 20×20 finishing faster than 10×10 is expected, not a bug.
- Throughput stays in the same band (~5.3k–6.5k sequential HTTP QPS). Larger maps take longer mainly because they need more steps, not because each request gets slower.
- Almost all wall time is HTTP round-trip. The PRNG decision itself is microseconds.
- Report JSON write is excluded from the wall-clock column and can exceed the walk time on large trajectories.
