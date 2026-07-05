use super::store::IntelligenceStore;
use std::sync::Arc;

pub struct FailurePattern {
    pub tool: String,
    pub error_snippet: String,
}

pub struct CrossSessionLearning {
    store: Arc<dyn IntelligenceStore>,
}

impl std::fmt::Debug for CrossSessionLearning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CrossSessionLearning").finish()
    }
}

impl CrossSessionLearning {
    pub fn new(store: Arc<dyn IntelligenceStore>) -> Self {
        Self { store }
    }

    pub async fn record_failure(&self, tool: &str, error: &str, judgment: &str) {
        let _ = self.store.save_pattern(tool, error, judgment).await;
    }

    pub async fn get_proactive_hints(&self, tool: &str) -> Vec<String> {
        self.store.get_patterns(tool).await.unwrap_or_default()
    }
}
