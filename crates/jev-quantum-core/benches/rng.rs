use criterion::{black_box, criterion_group, criterion_main, Criterion};
use indexmap::IndexMap;
use jev_quantum_core::protocol::{Question, SystemOneRequest};
use jev_quantum_core::rng::{fill_u64s, Xoshiro256PlusPlus};
use jev_quantum_core::{DecisionEngine, EngineConfig, RngMode};
use serde_json::json;

fn sample_request() -> SystemOneRequest {
    let mut questions = IndexMap::new();
    questions.insert(
        "move".to_string(),
        Question::Choice {
            instructions: None,
            criteria: IndexMap::from_iter([
                ("UP".into(), None),
                ("DOWN".into(), None),
                ("LEFT".into(), None),
                ("RIGHT".into(), None),
            ]),
        },
    );
    SystemOneRequest {
        model: "jev-quantum-latest".to_string(),
        state: json!("bench"),
        questions,
    }
}

fn bench_scalar(c: &mut Criterion) {
    c.bench_function("xoshiro256pp_u64", |b| {
        let mut rng = Xoshiro256PlusPlus::from_seed(1);
        b.iter(|| black_box(rng.next_u64()));
    });
}

fn bench_fill(c: &mut Criterion) {
    c.bench_function("fill_u64s_4096", |b| {
        let mut seeder = Xoshiro256PlusPlus::from_seed(2);
        let mut buf = vec![0u64; 4096];
        b.iter(|| {
            fill_u64s(&mut buf, &mut seeder);
            black_box(&buf);
        });
    });
}

fn bench_direct_engine(c: &mut Criterion) {
    let engine = DecisionEngine::new(EngineConfig {
        mode: RngMode::Direct,
        seed: 3,
        ..EngineConfig::default()
    });
    let req = sample_request();
    c.bench_function("engine_direct_choice4", |b| {
        b.iter(|| black_box(engine.decide(&req).unwrap()));
    });
}

fn bench_buffered_engine(c: &mut Criterion) {
    let engine = DecisionEngine::new(EngineConfig {
        mode: RngMode::Buffered,
        seed: 4,
        ..EngineConfig::default()
    });
    let req = sample_request();
    c.bench_function("engine_buffered_choice4", |b| {
        b.iter(|| black_box(engine.decide(&req).unwrap()));
    });
}

criterion_group!(
    benches,
    bench_scalar,
    bench_fill,
    bench_direct_engine,
    bench_buffered_engine
);
criterion_main!(benches);
