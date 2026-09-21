# Jev Quantum

[中文](README.md) | [English](README.en.md)

> The sub-microsecond System −1 model with Gaussian-distributed accuracy.

Jev Quantum is a Jev-compatible **random baseline**. It speaks the TypeSafe System One protocol (`noul` / `choice` / `score`), ignores the prompt, and answers from a fast PRNG. Use it as a dummy, a mock, or a lower bound for agent routing.

It is not a semantic model. The RNG is not cryptographic.

## One-minute start

```bash
cargo run --release -p jev-quantum-server
```

```bash
curl http://127.0.0.1:3000/v1/systemone \
  -H "Content-Type: application/json" \
  -d "{\"model\":\"jev-quantum-latest\",\"state\":\"I was charged twice.\",\"questions\":{\"refund\":{\"type\":\"noul\",\"instructions\":\"Refund?\"}}}"
```

Open `http://127.0.0.1:3000/demo/` and import a `*.summary.json` report. The page never calls an API.

## Compare with TypeSafe Jev

```bash
cp .env.example .env
# set AI_GATEWAY_API_KEY
cargo run --release -p jev-quantum-bench -- latency --targets local,jev --requests 100
cargo run --release -p jev-quantum-bench -- maze-record --targets local,jev --width 10 --height 10 --max-steps 200 --output reports/
```

TypeSafe curl shape:

```bash
curl https://api.typesafe.ai/v1/systemone \
  -H "Authorization: Bearer $AI_GATEWAY_API_KEY" \
  -H "Content-Type: application/json" \
  -d "{\"model\":\"jev-latest\",\"state\":\"I was charged twice.\",\"questions\":{\"refund\":{\"type\":\"noul\"}}}"
```

## Truth in advertising

| Number       | Meaning                      |
| ------------ | ---------------------------- |
| Core bench   | In-process PRNG + mapping    |
| `/metrics`   | Server handler time          |
| Bench report | Client-observed HTTP latency |

`direct` mode is the default. `buffered` keeps a worker-local random pool with background SIMD refill and synchronous fallback. Benchmark both before you believe either slogan.

## Docs

- [API](docs/api.md)
- [Benchmarking](docs/benchmarking.md)
- [Report schema](docs/report-schema.md)

## License

[MIT](LICENSE)
