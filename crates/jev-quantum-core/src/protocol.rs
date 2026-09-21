use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

pub const MAX_CHOICE_OPTIONS: usize = 255;
pub const MIN_SCORE_LEVELS: usize = 2;
pub const MAX_SCORE_LEVELS: usize = 10;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SystemOneRequest {
    pub model: String,
    pub state: Value,
    pub questions: IndexMap<String, Question>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Question {
    Noul {
        #[serde(default)]
        instructions: Option<String>,
        #[serde(default)]
        criteria: Option<NoulCriteria>,
    },
    Choice {
        #[serde(default)]
        instructions: Option<String>,
        criteria: IndexMap<String, Option<String>>,
    },
    Score {
        #[serde(default)]
        instructions: Option<String>,
        criteria: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct NoulCriteria {
    #[serde(rename = "true", default)]
    pub true_desc: Option<String>,
    #[serde(rename = "false", default)]
    pub false_desc: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SystemOneResponse {
    pub model: String,
    pub answers: IndexMap<String, Answer>,
    pub usage: Usage,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Answer {
    Noul {
        noul: f64,
    },
    Choice {
        choice: String,
        confidence: f64,
        probabilities: IndexMap<String, f64>,
    },
    Score {
        score: f64,
        confidence: f64,
        legend: IndexMap<String, String>,
        probabilities: IndexMap<String, f64>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    pub error: ErrorBody,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub message: String,
    pub code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelsResponse {
    pub object: String,
    pub data: Vec<ModelInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub object: String,
    pub owned_by: String,
    #[serde(default)]
    pub created: u64,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    #[error("model must not be empty")]
    MissingModel,
    #[error("questions must not be empty")]
    EmptyQuestions,
    #[error("choice '{0}' must have between 1 and {MAX_CHOICE_OPTIONS} options")]
    InvalidChoiceCardinality(String),
    #[error("score '{0}' must have between {MIN_SCORE_LEVELS} and {MAX_SCORE_LEVELS} levels")]
    InvalidScoreCardinality(String),
}

impl ProtocolError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::MissingModel | Self::EmptyQuestions => "invalid_request",
            Self::InvalidChoiceCardinality(_) | Self::InvalidScoreCardinality(_) => {
                "invalid_cardinality"
            }
        }
    }

    pub fn into_api_error(self) -> ApiError {
        ApiError {
            error: ErrorBody {
                message: self.to_string(),
                code: self.code().to_string(),
            },
        }
    }
}

impl Question {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Noul { .. } => "noul",
            Self::Choice { .. } => "choice",
            Self::Score { .. } => "score",
        }
    }
}

impl Answer {
    pub fn choice_label(&self) -> Option<&str> {
        match self {
            Self::Choice { choice, .. } => Some(choice.as_str()),
            _ => None,
        }
    }
}

impl SystemOneRequest {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.model.trim().is_empty() {
            return Err(ProtocolError::MissingModel);
        }
        if self.questions.is_empty() {
            return Err(ProtocolError::EmptyQuestions);
        }
        for (id, question) in &self.questions {
            match question {
                Question::Choice { criteria, .. } => {
                    if criteria.is_empty() || criteria.len() > MAX_CHOICE_OPTIONS {
                        return Err(ProtocolError::InvalidChoiceCardinality(id.clone()));
                    }
                }
                Question::Score { criteria, .. } => {
                    if criteria.len() < MIN_SCORE_LEVELS || criteria.len() > MAX_SCORE_LEVELS {
                        return Err(ProtocolError::InvalidScoreCardinality(id.clone()));
                    }
                }
                Question::Noul { .. } => {}
            }
        }
        Ok(())
    }
}

pub fn models_catalog(model_id: &str) -> ModelsResponse {
    ModelsResponse {
        object: "list".to_string(),
        data: vec![ModelInfo {
            id: model_id.to_string(),
            object: "model".to_string(),
            owned_by: "jev-quantum".to_string(),
            created: 0,
        }],
    }
}
