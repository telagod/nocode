use crate::message::QueryMessage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptRole {
    Conversation,
    ToolRequest,
    ToolProgress,
    ToolResult,
    ToolMessage,
}

impl TranscriptRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Conversation => "conversation",
            Self::ToolRequest => "tool_request",
            Self::ToolProgress => "tool_progress",
            Self::ToolResult => "tool_result",
            Self::ToolMessage => "tool_message",
        }
    }

    pub fn parse_kind(value: &str) -> Option<Self> {
        match value {
            "conversation" => Some(Self::Conversation),
            "tool_request" => Some(Self::ToolRequest),
            "tool_progress" => Some(Self::ToolProgress),
            "tool_result" => Some(Self::ToolResult),
            "tool_message" => Some(Self::ToolMessage),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptEntry {
    pub turn: u32,
    pub role: TranscriptRole,
    pub content: String,
}

impl TranscriptEntry {
    pub fn new(turn: u32, role: TranscriptRole, content: impl Into<String>) -> Self {
        Self {
            turn,
            role,
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QueryTranscript {
    pub entries: Vec<TranscriptEntry>,
}

impl QueryTranscript {
    pub fn from_messages(messages: &[QueryMessage], turn: u32) -> Self {
        let mut transcript = Self::default();
        for message in messages {
            transcript.push(turn, TranscriptRole::Conversation, message.summary());
        }
        transcript
    }

    pub fn push(&mut self, turn: u32, role: TranscriptRole, content: impl Into<String>) {
        self.entries.push(TranscriptEntry::new(turn, role, content));
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::{QueryTranscript, TranscriptRole};
    use crate::message::QueryMessage;

    #[test]
    fn transcript_is_seeded_from_messages() {
        let transcript = QueryTranscript::from_messages(
            &[QueryMessage::system("seed"), QueryMessage::user("prompt")],
            1,
        );
        assert_eq!(transcript.len(), 2);
        assert_eq!(transcript.entries[0].turn, 1);
        assert_eq!(transcript.entries[0].role, TranscriptRole::Conversation);
        assert_eq!(transcript.entries[1].content, "user: prompt");
    }

    #[test]
    fn transcript_role_labels_are_stable() {
        assert_eq!(TranscriptRole::ToolResult.as_str(), "tool_result");
        assert_eq!(TranscriptRole::ToolMessage.as_str(), "tool_message");
        assert_eq!(
            TranscriptRole::parse_kind("tool_progress"),
            Some(TranscriptRole::ToolProgress)
        );
        assert_eq!(TranscriptRole::parse_kind("missing"), None);
    }
}
