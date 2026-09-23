# Jev Quantum

[中文](README.md) | [English](README.en.md)

![cover](./assets/images/cover.png)

> 亚微秒级 System −1 模型，准确率服从高斯分布。

Jev Quantum 是兼容 Jev 协议的**随机基线**。它讲 TypeSafe System One 的话（`noul` / `choice` / `score`），不读提示词，答案来自高速伪随机数。拿它当 dummy、mock，或 Agent 路由评测的下界。

它不是语义模型。随机数也不是密码学安全的。

## 一分钟上手

```bash
cargo run --release -p jev-quantum-server
```

```bash
curl http://127.0.0.1:3000/v1/systemone \
  -H "Content-Type: application/json" \
  -d "{\"model\":\"jev-quantum-latest\",\"state\":\"I was charged twice.\",\"questions\":{\"refund\":{\"type\":\"noul\",\"instructions\":\"Refund?\"}}}"
```

打开 `http://127.0.0.1:3000/demo/`，导入一份 `*.summary.json` 报告。页面不会调用任何 API，支持随机、Jev、Flood Fill、Jev Memory v1/v2、Jev Flood Fill、Luna Session 七版本同屏回放。

## 和 TypeSafe Jev 对比

```bash
cp .env.example .env
# 填写 AI_GATEWAY_API_KEY
cargo run --release -p jev-quantum-bench -- latency --targets local,jev --requests 100
cargo run --release -p jev-quantum-bench -- maze-record --targets local,jev --width 10 --height 10 --max-steps 200 --output reports/
```

新增迷宫策略可用 `--targets flood-fill,jev-memory,jev-flood-fill`：前者在线感知墙壁并重算 BFS 距离场，Jev Memory 使用定长空间记忆，Jev Flood Fill 根据本地更新的 BFS 距离场选择动作。用 `--maze-seed` 固定基线，详见 [Benchmarking](docs/benchmarking.md)。

官方接口形态：

```bash
curl https://api.typesafe.ai/v1/systemone \
  -H "Authorization: Bearer $AI_GATEWAY_API_KEY" \
  -H "Content-Type: application/json" \
  -d "{\"model\":\"jev-latest\",\"state\":\"I was charged twice.\",\"questions\":{\"refund\":{\"type\":\"noul\"}}}"
```

## 数字怎么读

| 数字       | 含义                   |
| ---------- | ---------------------- |
| Core bench | 进程内 PRNG + 映射     |
| `/metrics` | 服务端 handler 耗时    |
| Bench 报告 | 客户端观测到的 HTTP 延迟 |

默认 `direct` 模式。`buffered` 使用 worker 本地随机池，后台 SIMD 补水，同步路径可降级。两种口号都先跑完再信。

## 文档

- [API](docs/api.md)
- [Benchmarking](docs/benchmarking.md)
- [Report schema](docs/report-schema.md)

## 许可

[MIT](LICENSE)
