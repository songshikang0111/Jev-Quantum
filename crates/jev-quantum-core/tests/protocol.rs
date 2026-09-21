use indexmap::IndexMap;
use jev_quantum_core::protocol::{
    models_catalog, ProtocolError, Question, SystemOneRequest, SystemOneResponse,
};
use jev_quantum_core::{DecisionEngine, EngineConfig};
use serde_json::json;

fn load(name: &str) -> String {
    std::fs::read_to_string(format!("tests/fixtures/{name}")).expect("fixture")
}

#[test]
fn noul_request_roundtrip() {
    let raw = load("noul_request.json");
    let parsed: SystemOneRequest = serde_json::from_str(&raw).unwrap();
    parsed.validate().unwrap();
    assert_eq!(parsed.questions.len(), 1);
    assert_eq!(parsed.questions["refund"].kind(), "noul");
    let again = serde_json::to_value(&parsed).unwrap();
    let original: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(again["model"], original["model"]);
    assert_eq!(again["questions"]["refund"]["type"], "noul");
}

#[test]
fn choice_request_roundtrip() {
    let parsed: SystemOneRequest = serde_json::from_str(&load("choice_request.json")).unwrap();
    parsed.validate().unwrap();
    match &parsed.questions["department"] {
        Question::Choice { criteria, .. } => {
            assert_eq!(criteria.len(), 3);
            assert!(criteria.contains_key("billing"));
        }
        other => panic!("expected choice, got {other:?}"),
    }
}

#[test]
fn score_request_roundtrip() {
    let parsed: SystemOneRequest = serde_json::from_str(&load("score_request.json")).unwrap();
    parsed.validate().unwrap();
    match &parsed.questions["urgency"] {
        Question::Score { criteria, .. } => assert_eq!(criteria.len(), 3),
        other => panic!("expected score, got {other:?}"),
    }
}

#[test]
fn mixed_response_roundtrip() {
    let raw = load("mixed_response.json");
    let parsed: SystemOneResponse = serde_json::from_str(&raw).unwrap();
    assert_eq!(parsed.answers.len(), 3);
    let encoded = serde_json::to_value(&parsed).unwrap();
    assert_eq!(encoded["answers"]["refund"]["noul"], 0.99);
    assert_eq!(encoded["answers"]["department"]["choice"], "billing");
    assert_eq!(encoded["answers"]["urgency"]["score"], 2.0);
}

#[test]
fn missing_model_rejected() {
    let req = SystemOneRequest {
        model: "   ".to_string(),
        state: json!("x"),
        questions: IndexMap::from_iter([(
            "q".to_string(),
            Question::Noul {
                instructions: None,
                criteria: None,
            },
        )]),
    };
    assert_eq!(req.validate(), Err(ProtocolError::MissingModel));
}

#[test]
fn empty_questions_rejected() {
    let req = SystemOneRequest {
        model: "jev-quantum-latest".to_string(),
        state: json!("x"),
        questions: IndexMap::new(),
    };
    assert_eq!(req.validate(), Err(ProtocolError::EmptyQuestions));
}

#[test]
fn choice_cardinality_rejected() {
    let req = SystemOneRequest {
        model: "jev-quantum-latest".to_string(),
        state: json!("x"),
        questions: IndexMap::from_iter([(
            "too_many".to_string(),
            Question::Choice {
                instructions: None,
                criteria: (0..256).map(|i| (i.to_string(), None)).collect(),
            },
        )]),
    };
    assert!(matches!(
        req.validate(),
        Err(ProtocolError::InvalidChoiceCardinality(_))
    ));
}

#[test]
fn score_cardinality_rejected() {
    let req = SystemOneRequest {
        model: "jev-quantum-latest".to_string(),
        state: json!("x"),
        questions: IndexMap::from_iter([(
            "one".to_string(),
            Question::Score {
                instructions: None,
                criteria: vec!["only".to_string()],
            },
        )]),
    };
    assert!(matches!(
        req.validate(),
        Err(ProtocolError::InvalidScoreCardinality(_))
    ));
}

#[test]
fn models_catalog_lists_local_model() {
    let catalog = models_catalog("jev-quantum-latest");
    assert_eq!(catalog.object, "list");
    assert_eq!(catalog.data[0].id, "jev-quantum-latest");
}

#[test]
fn engine_answers_all_question_types() {
    let raw = load("mixed_request.json");
    let req: SystemOneRequest = serde_json::from_str(&raw).unwrap();
    let engine = DecisionEngine::new(EngineConfig {
        seed: 42,
        ..EngineConfig::default()
    });
    let response = engine.decide(&req).unwrap();
    assert_eq!(response.answers.len(), 3);
    assert!(response.answers.contains_key("refund"));
    assert!(response.answers.contains_key("department"));
    assert!(response.answers.contains_key("urgency"));
}
