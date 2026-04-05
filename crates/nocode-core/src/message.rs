#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryMessageRole {
    System,
    User,
    Assistant,
    Tool,
}

impl QueryMessageRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "system" => Some(Self::System),
            "user" => Some(Self::User),
            "assistant" => Some(Self::Assistant),
            "tool" => Some(Self::Tool),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryMessage {
    pub role: QueryMessageRole,
    pub content: String,
}

impl QueryMessage {
    pub fn new(role: QueryMessageRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self::new(QueryMessageRole::System, content)
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::new(QueryMessageRole::User, content)
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self::new(QueryMessageRole::Assistant, content)
    }

    pub fn tool(content: impl Into<String>) -> Self {
        Self::new(QueryMessageRole::Tool, content)
    }

    pub fn summary(&self) -> String {
        format!("{}: {}", self.role.as_str(), self.content)
    }
}

#[cfg(test)]
mod tests {
    use super::{QueryMessage, QueryMessageRole};

    #[test]
    fn query_message_helpers_preserve_roles() {
        let user = QueryMessage::user("continue rewrite");
        let tool = QueryMessage::tool("tool-result");
        assert_eq!(user.role, QueryMessageRole::User);
        assert_eq!(tool.role, QueryMessageRole::Tool);
        assert_eq!(user.summary(), "user: continue rewrite");
    }
}
