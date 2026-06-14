pub mod openai;
pub mod stream;

/// Wire protocol format for LLM API requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    OpenAI,
    Anthropic,
    Gemini,
}

impl Protocol {
    pub fn parse_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "anthropic" => Protocol::Anthropic,
            "gemini" => Protocol::Gemini,
            _ => Protocol::OpenAI,
        }
    }
}
