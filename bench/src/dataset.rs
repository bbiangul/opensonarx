use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkDataset {
    pub name: String,
    pub description: String,
    pub cases: Vec<BenchmarkCase>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkCase {
    pub id: String,
    pub query: String,
    pub difficulty: Difficulty,
    pub relevant_docs: Vec<RelevantDoc>,
    pub metadata: CaseMetadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Difficulty {
    Easy,
    Medium,
    Hard,
    Adversarial,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelevantDoc {
    pub doc_id: String,
    pub title: String,
    pub content: String,
    pub relevance: u8, // 0-3 graded relevance
    /// If Some, this doc is an image (description used for fallback text embedding)
    #[serde(default)]
    pub image_description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseMetadata {
    pub region: String,
    pub language: String,
    pub topic: String,
    pub content_type: String,
    /// "text-to-text" or "text-to-image"
    #[serde(default = "default_modality")]
    pub modality: String,
}

fn default_modality() -> String {
    "text-to-text".to_string()
}

impl std::fmt::Display for Difficulty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Difficulty::Easy => write!(f, "easy"),
            Difficulty::Medium => write!(f, "medium"),
            Difficulty::Hard => write!(f, "hard"),
            Difficulty::Adversarial => write!(f, "adversarial"),
        }
    }
}
