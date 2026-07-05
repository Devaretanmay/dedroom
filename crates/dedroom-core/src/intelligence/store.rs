use async_trait::async_trait;


#[async_trait]
pub trait IntelligenceStore: Send + Sync {
    async fn save_pattern(&self, tool: &str, error: &str, judgment: &str) -> anyhow::Result<()>;
    async fn get_patterns(&self, tool: &str) -> anyhow::Result<Vec<String>>;
    async fn update_trust_score(&self, agent_id: &str, delta: f64) -> anyhow::Result<()>;
    async fn get_trust_score(&self, agent_id: &str) -> anyhow::Result<f64>;
    async fn get_global_stats(&self) -> anyhow::Result<serde_json::Value>;
}

pub struct InMemoryIntelligenceStore {
    patterns: std::sync::Mutex<Vec<(String, String, String)>>,
    trust_scores: std::sync::Mutex<std::collections::HashMap<String, f64>>,
}

impl InMemoryIntelligenceStore {
    pub fn new() -> Self {
        Self {
            patterns: std::sync::Mutex::new(Vec::new()),
            trust_scores: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }
}

#[async_trait]
impl IntelligenceStore for InMemoryIntelligenceStore {
    async fn save_pattern(&self, tool: &str, error: &str, judgment: &str) -> anyhow::Result<()> {
        let mut patterns = self.patterns.lock().unwrap();
        patterns.push((tool.to_string(), error.to_string(), judgment.to_string()));
        Ok(())
    }

    async fn get_patterns(&self, tool: &str) -> anyhow::Result<Vec<String>> {
        let patterns = self.patterns.lock().unwrap();
        Ok(patterns.iter()
            .filter(|(t, _, _)| t == tool)
            .map(|(_, _, j)| j.clone())
            .collect())
    }

    async fn update_trust_score(&self, agent_id: &str, delta: f64) -> anyhow::Result<()> {
        let mut scores = self.trust_scores.lock().unwrap();
        let score = scores.entry(agent_id.to_string()).or_insert(100.0);
        *score += delta;
        Ok(())
    }

    async fn get_trust_score(&self, agent_id: &str) -> anyhow::Result<f64> {
        let scores = self.trust_scores.lock().unwrap();
        Ok(scores.get(agent_id).copied().unwrap_or(100.0))
    }

    async fn get_global_stats(&self) -> anyhow::Result<serde_json::Value> {
        let scores = self.trust_scores.lock().unwrap();
        let mut total_score = 0.0;
        let mut count = 0;
        for score in scores.values() {
            total_score += score;
            count += 1;
        }
        let avg_trust = if count > 0 { total_score / count as f64 } else { 100.0 };
        Ok(serde_json::json!({
            "average_trust_score": avg_trust,
            "total_agents_tracked": count,
        }))
    }
}

#[cfg(feature = "sqlite")]
pub struct SqliteIntelligenceStore {
    conn: std::sync::Mutex<rusqlite::Connection>,
}

#[cfg(feature = "sqlite")]
impl SqliteIntelligenceStore {
    pub fn new(path: &str) -> anyhow::Result<Self> {
        let conn = rusqlite::Connection::open(path)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS patterns (
                id INTEGER PRIMARY KEY,
                tool TEXT NOT NULL,
                error TEXT NOT NULL,
                judgment TEXT NOT NULL
            )",
            [],
        )?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS trust_scores (
                agent_id TEXT PRIMARY KEY,
                score REAL NOT NULL
            )",
            [],
        )?;
        Ok(Self {
            conn: std::sync::Mutex::new(conn),
        })
    }
}

#[cfg(feature = "sqlite")]
#[async_trait]
impl IntelligenceStore for SqliteIntelligenceStore {
    async fn save_pattern(&self, tool: &str, error: &str, judgment: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO patterns (tool, error, judgment) VALUES (?1, ?2, ?3)",
            [tool, error, judgment],
        )?;
        Ok(())
    }

    async fn get_patterns(&self, tool: &str) -> anyhow::Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT judgment FROM patterns WHERE tool = ?1")?;
        let rows = stmt.query_map([tool], |row| row.get(0))?;
        let mut patterns = Vec::new();
        for r in rows {
            if let Ok(p) = r {
                patterns.push(p);
            }
        }
        Ok(patterns)
    }

    async fn update_trust_score(&self, agent_id: &str, delta: f64) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO trust_scores (agent_id, score) VALUES (?1, ?2)
             ON CONFLICT(agent_id) DO UPDATE SET score = score + ?2",
            rusqlite::params![agent_id, delta],
        )?;
        Ok(())
    }

    async fn get_trust_score(&self, agent_id: &str) -> anyhow::Result<f64> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT score FROM trust_scores WHERE agent_id = ?1")?;
        let mut rows = stmt.query([agent_id])?;
        if let Some(row) = rows.next()? {
            Ok(row.get(0)?)
        } else {
            Ok(100.0) // default score
        }
    }

    async fn get_global_stats(&self) -> anyhow::Result<serde_json::Value> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT AVG(score), COUNT(*) FROM trust_scores")?;
        let mut rows = stmt.query([])?;
        let (avg_trust, count): (f64, i64) = if let Some(row) = rows.next()? {
            (row.get(0).unwrap_or(100.0), row.get(1).unwrap_or(0))
        } else {
            (100.0, 0)
        };
        Ok(serde_json::json!({
            "average_trust_score": avg_trust,
            "total_agents_tracked": count,
        }))
    }
}
