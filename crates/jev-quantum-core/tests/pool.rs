use std::thread;
use std::time::Duration;

use indexmap::IndexMap;
use jev_quantum_core::protocol::{Question, SystemOneRequest};
use jev_quantum_core::rng::Entropy;
use jev_quantum_core::{DecisionEngine, EngineConfig, RngMode};
use serde_json::json;

#[test]
fn buffered_pool_recovers_after_burst() {
    let engine = DecisionEngine::new(EngineConfig {
        mode: RngMode::Buffered,
        seed: 99,
        chunk_len: 16,
        chunk_count: 4,
        low_watermark: 4,
        refill_threads: 1,
        model_id: "jev-quantum-latest".to_string(),
    });

    let mut questions = IndexMap::new();
    questions.insert(
        "ok".to_string(),
        Question::Noul {
            instructions: None,
            criteria: None,
        },
    );
    let req = SystemOneRequest {
        model: "jev-quantum-latest".to_string(),
        state: json!("burst"),
        questions,
    };

    for _ in 0..2_000 {
        engine.decide(&req).unwrap();
    }
    thread::sleep(Duration::from_millis(20));
    for _ in 0..64 {
        engine.decide(&req).unwrap();
    }

    let stats = engine.stats();
    assert!(
        stats.hits + stats.fallbacks > 0,
        "engine should consume entropy"
    );
}

#[test]
fn concurrent_streams_are_isolated() {
    let engine = DecisionEngine::new(EngineConfig {
        seed: 1234,
        ..EngineConfig::default()
    });
    let mut questions = IndexMap::new();
    questions.insert(
        "coin".to_string(),
        Question::Noul {
            instructions: None,
            criteria: None,
        },
    );
    let req = SystemOneRequest {
        model: "jev-quantum-latest".to_string(),
        state: json!("iso"),
        questions,
    };

    thread::scope(|scope| {
        let left = scope.spawn(|| {
            (0..32)
                .map(|_| engine.decide(&req).unwrap())
                .collect::<Vec<_>>()
        });
        let right = scope.spawn(|| {
            (0..32)
                .map(|_| engine.decide(&req).unwrap())
                .collect::<Vec<_>>()
        });
        let a = left.join().unwrap();
        let b = right.join().unwrap();
        assert_ne!(a, b, "independent streams should diverge");
    });
}

#[test]
fn bounded_index_stays_in_range() {
    let mut rng = jev_quantum_core::rng::Xoshiro256PlusPlus::from_seed(5);
    for n in [1usize, 2, 3, 4, 7, 255] {
        for _ in 0..256 {
            let idx = rng.bounded_usize(n);
            assert!(idx < n, "{idx} >= {n}");
        }
    }
}
