use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use indexmap::IndexMap;

use crate::pool::{PoolHandle, RngStats, RngStatsSnapshot, WorkerRng};
use crate::protocol::{
    Answer, ProtocolError, Question, SystemOneRequest, SystemOneResponse, Usage,
};
use crate::rng::{detect_backend, Backend, Entropy};
use crate::stream_seed;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RngMode {
    Direct,
    Buffered,
}

impl RngMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Buffered => "buffered",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "direct" => Some(Self::Direct),
            "buffered" => Some(Self::Buffered),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub mode: RngMode,
    pub seed: u64,
    pub chunk_len: usize,
    pub chunk_count: usize,
    pub low_watermark: usize,
    pub refill_threads: usize,
    pub model_id: String,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            mode: RngMode::Direct,
            seed: crate::process_seed(),
            chunk_len: 4096,
            chunk_count: 16,
            low_watermark: 1024,
            refill_threads: 1,
            model_id: "jev-quantum-latest".to_string(),
        }
    }
}

struct WorkerSlot {
    rng: WorkerRng,
}

thread_local! {
    static WORKER: RefCell<Option<WorkerSlot>> = const { RefCell::new(None) };
}

pub struct DecisionEngine {
    config: EngineConfig,
    next_stream: AtomicU64,
    pool: Option<PoolHandle>,
    stats: Arc<RngStats>,
    backend: Backend,
}

impl DecisionEngine {
    pub fn new(config: EngineConfig) -> Self {
        let stats = Arc::new(RngStats::default());
        let backend = detect_backend();
        let pool = match config.mode {
            RngMode::Buffered => Some(PoolHandle::new(
                stream_seed(config.seed, 0xB0FF_E5ED),
                config.chunk_len,
                config.chunk_count,
                config.refill_threads,
                Arc::clone(&stats),
            )),
            RngMode::Direct => None,
        };
        Self {
            config,
            next_stream: AtomicU64::new(1),
            pool,
            stats,
            backend,
        }
    }

    pub fn backend(&self) -> Backend {
        self.backend
    }

    pub fn mode(&self) -> RngMode {
        self.config.mode
    }

    pub fn model_id(&self) -> &str {
        &self.config.model_id
    }

    pub fn stats(&self) -> RngStatsSnapshot {
        self.stats.snapshot()
    }

    pub fn decide(&self, request: &SystemOneRequest) -> Result<SystemOneResponse, ProtocolError> {
        request.validate()?;
        Ok(self.with_worker(|rng| {
            let mut answers = IndexMap::with_capacity(request.questions.len());
            for (id, question) in &request.questions {
                answers.insert(id.clone(), answer_question(question, rng));
            }
            SystemOneResponse {
                model: self.config.model_id.clone(),
                answers,
                usage: Usage {
                    input_tokens: 0,
                    output_tokens: 0,
                },
            }
        }))
    }

    pub fn decide_with<R: Entropy>(
        &self,
        request: &SystemOneRequest,
        rng: &mut R,
    ) -> Result<SystemOneResponse, ProtocolError> {
        request.validate()?;
        let mut answers = IndexMap::with_capacity(request.questions.len());
        for (id, question) in &request.questions {
            answers.insert(id.clone(), answer_question(question, rng));
        }
        Ok(SystemOneResponse {
            model: self.config.model_id.clone(),
            answers,
            usage: Usage::default(),
        })
    }

    fn with_worker<T>(&self, f: impl FnOnce(&mut dyn Entropy) -> T) -> T {
        WORKER.with(|slot| {
            let mut guard = slot.borrow_mut();
            if guard.is_none() {
                *guard = Some(self.spawn_worker());
            }
            let worker = guard.as_mut().expect("worker initialized");
            f(&mut worker.rng)
        })
    }

    fn spawn_worker(&self) -> WorkerSlot {
        let stream = self.next_stream.fetch_add(1, Ordering::Relaxed);
        let seed = stream_seed(self.config.seed, stream);
        let rng = match (&self.config.mode, &self.pool) {
            (RngMode::Buffered, Some(pool)) => WorkerRng::buffered(
                seed,
                Arc::clone(&pool.coordinator),
                Arc::clone(&self.stats),
                self.config.low_watermark,
            ),
            _ => WorkerRng::direct(seed, Arc::clone(&self.stats)),
        };
        WorkerSlot { rng }
    }
}

fn answer_question(question: &Question, rng: &mut dyn Entropy) -> Answer {
    match question {
        Question::Noul { .. } => Answer::Noul {
            noul: rng.next_f64(),
        },
        Question::Choice { criteria, .. } => {
            let idx = rng.bounded_usize(criteria.len());
            let (chosen, _) = criteria
                .get_index(idx)
                .expect("validated choice is non-empty");
            let mut probabilities = IndexMap::with_capacity(criteria.len());
            for key in criteria.keys() {
                probabilities.insert(key.clone(), if key == chosen { 1.0 } else { 0.0 });
            }
            Answer::Choice {
                choice: chosen.clone(),
                confidence: 1.0,
                probabilities,
            }
        }
        Question::Score { criteria, .. } => {
            let idx = rng.bounded_usize(criteria.len());
            let mut legend = IndexMap::with_capacity(criteria.len());
            let mut probabilities = IndexMap::with_capacity(criteria.len());
            for (i, label) in criteria.iter().enumerate() {
                let key = i.to_string();
                legend.insert(key.clone(), label.clone());
                probabilities.insert(key, if i == idx { 1.0 } else { 0.0 });
            }
            Answer::Score {
                score: idx as f64,
                confidence: 1.0,
                legend,
                probabilities,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Xoshiro256PlusPlus;
    use serde_json::json;

    fn sample_request() -> SystemOneRequest {
        let mut questions = IndexMap::new();
        questions.insert(
            "move".to_string(),
            Question::Choice {
                instructions: Some("pick a direction".to_string()),
                criteria: IndexMap::from_iter([
                    ("UP".to_string(), Some("up".to_string())),
                    ("DOWN".to_string(), Some("down".to_string())),
                    ("LEFT".to_string(), Some("left".to_string())),
                    ("RIGHT".to_string(), Some("right".to_string())),
                ]),
            },
        );
        SystemOneRequest {
            model: "jev-quantum-latest".to_string(),
            state: json!({"x": 0, "y": 0}),
            questions,
        }
    }

    #[test]
    fn seeded_engine_is_deterministic() {
        let req = sample_request();
        let mut a = Xoshiro256PlusPlus::from_seed(7);
        let mut b = Xoshiro256PlusPlus::from_seed(7);
        let engine = DecisionEngine::new(EngineConfig {
            seed: 7,
            ..EngineConfig::default()
        });
        let left = engine.decide_with(&req, &mut a).unwrap();
        let right = engine.decide_with(&req, &mut b).unwrap();
        assert_eq!(left, right);
    }

    #[test]
    fn empty_questions_rejected() {
        let engine = DecisionEngine::new(EngineConfig::default());
        let req = SystemOneRequest {
            model: "jev-quantum-latest".to_string(),
            state: json!("x"),
            questions: IndexMap::new(),
        };
        assert_eq!(
            engine.decide(&req).unwrap_err(),
            ProtocolError::EmptyQuestions
        );
    }
}
