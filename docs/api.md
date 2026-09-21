# Jev Quantum API

Local server speaks the TypeSafe System One protocol.

## Endpoints

| Method | Path | Purpose |
| --- | --- | --- |
| `POST` | `/v1/systemone` | Evaluate `noul`, `choice`, and `score` questions |
| `GET` | `/v1/models` | List the local model id |
| `GET` | `/health` | Liveness and RNG backend |
| `GET` | `/metrics` | Prometheus text |
| `GET` | `/demo` | Offline maze replay page |

## Request

```json
{
  "model": "jev-quantum-latest",
  "state": "I was charged twice.",
  "questions": {
    "refund": { "type": "noul", "instructions": "Refund?" },
    "queue": {
      "type": "choice",
      "criteria": { "billing": "pay", "technical": "bug" }
    },
    "urgency": {
      "type": "score",
      "criteria": ["Can wait", "This week", "Today"]
    }
  }
}
```

Constraints:

- `choice` cardinality is 1–255
- `score` levels are 2–10
- `state` may be a string, object, or array
- empty `questions` or empty `model` returns HTTP 422

## Response

Answers keep option order. Quantum returns one-hot probabilities and `confidence = 1.0` for choice/score. Token usage is always zero.

This is a random baseline, not a semantic model. The generator is not cryptographic.
