#[derive(Debug, Clone, Copy)]
pub struct JudgmentVector {
    pub complexity: f64,
    pub confidence: f64,
    pub success: bool,
}

#[derive(Debug)]
pub struct JudgmentPreservation {
    vectors: Vec<JudgmentVector>,
}

impl JudgmentPreservation {
    pub fn new() -> Self {
        Self { vectors: Vec::new() }
    }

    pub fn extract_reflection(&mut self, response_text: &str) -> Option<JudgmentVector> {
        let mut is_reflective = false;
        let mut complexity: f64 = 0.1;
        let mut confidence: f64 = 0.5;
        
        let text_lower = response_text.to_lowercase();
        
        // Check for common reflection tags
        if text_lower.contains("<thinking>") || text_lower.contains("<!--") {
            is_reflective = true;
            complexity += 0.4;
        }

        // Check for reflection keywords
        let keywords = ["i should", "perhaps", "however", "let's try", "i noticed", "i made a mistake"];
        let mut keyword_count = 0;
        for kw in keywords {
            if text_lower.contains(kw) {
                is_reflective = true;
                keyword_count += 1;
            }
        }
        
        // Increase complexity based on keyword density
        complexity += (keyword_count as f64 * 0.1).min(0.5);

        // Analyze confidence
        if text_lower.contains("i am sure") || text_lower.contains("successfully") || text_lower.contains("fixed") {
            confidence += 0.3;
        } else if text_lower.contains("i'm not sure") || text_lower.contains("failed") || text_lower.contains("error") {
            confidence -= 0.3;
        }

        if is_reflective {
            let vec = JudgmentVector {
                complexity: complexity.min(1.0),
                confidence: confidence.clamp(0.0, 1.0),
                success: !text_lower.contains("error"),
            };
            self.vectors.push(vec.clone());
            Some(vec)
        } else {
            None
        }
    }
}
