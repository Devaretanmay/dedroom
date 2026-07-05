use super::store::IntelligenceStore;
use std::sync::Arc;

pub struct AgentTrustScore {
    pub agent_id: String,
    pub score: f64,
}

pub struct TrustVerification {
    pub store: Arc<dyn IntelligenceStore>,
}

impl std::fmt::Debug for TrustVerification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrustVerification").finish()
    }
}

impl TrustVerification {
    pub fn new(store: Arc<dyn IntelligenceStore>) -> Self {
        Self { store }
    }

    pub async fn update_score(&self, agent_id: &str, is_success: bool) {
        let delta = if is_success { 1.0 } else { -2.0 };
        let _ = self.store.update_trust_score(agent_id, delta).await;
    }

    pub async fn get_score(&self, agent_id: &str) -> f64 {
        self.store.get_trust_score(agent_id).await.unwrap_or(100.0)
    }

    pub async fn is_trusted(&self, agent_id: &str) -> bool {
        self.get_score(agent_id).await > 50.0
    }
}
