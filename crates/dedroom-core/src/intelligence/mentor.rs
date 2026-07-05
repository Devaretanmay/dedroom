#[derive(Debug)]
pub struct MentorMode {
    pub enabled: bool,
}

impl MentorMode {
    pub fn new(enabled: bool) -> Self {
        Self { enabled }
    }

    pub fn generate_coaching_hint(&self, tilt_index: f64) -> Option<String> {
        if !self.enabled {
            return None;
        }

        if tilt_index > 0.8 {
            Some("Take a step back. You are repeatedly trying failing actions. Review the documentation or try a completely different approach.".into())
        } else if tilt_index > 0.5 {
            Some("You seem to be stuck. Consider searching the codebase for examples of how to do this correctly.".into())
        } else {
            None
        }
    }

    pub fn post_session_reflection(&self, blocks: usize, errors: usize) -> Option<String> {
        if !self.enabled || (blocks == 0 && errors == 0) {
            return None;
        }
        
        Some(format!("Session ended with {} blocked loops and {} errors. I should reflect on these failures and avoid repeating them in future sessions.", blocks, errors))
    }
}
