use std::collections::HashMap;

/// A structured task description that can be validated before execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskPacket {
    pub goal: String,
    pub constraints: Vec<String>,
    pub target_files: Vec<String>,
    pub budget: Option<TaskBudgetSpec>,
    pub priority: TaskPriority,
    pub labels: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TaskPriority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskBudgetSpec {
    pub max_turns: Option<u32>,
    pub max_tokens: Option<u64>,
    pub max_tool_calls: Option<u32>,
}

/// Validation errors for a task packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskPacketError {
    EmptyGoal,
    GoalTooLong { len: usize, max: usize },
    TooManyConstraints { count: usize, max: usize },
    TooManyFiles { count: usize, max: usize },
    InvalidBudget { reason: String },
}

impl std::fmt::Display for TaskPacketError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyGoal => write!(f, "task goal must not be empty"),
            Self::GoalTooLong { len, max } => {
                write!(f, "goal is {len} chars, max {max}")
            }
            Self::TooManyConstraints { count, max } => {
                write!(f, "{count} constraints exceed max {max}")
            }
            Self::TooManyFiles { count, max } => {
                write!(f, "{count} target files exceed max {max}")
            }
            Self::InvalidBudget { reason } => write!(f, "invalid budget: {reason}"),
        }
    }
}

/// Configuration for task packet validation limits.
#[derive(Debug, Clone)]
pub struct TaskPacketLimits {
    pub max_goal_len: usize,
    pub max_constraints: usize,
    pub max_target_files: usize,
}

impl Default for TaskPacketLimits {
    fn default() -> Self {
        Self {
            max_goal_len: 2000,
            max_constraints: 20,
            max_target_files: 50,
        }
    }
}

impl TaskPacket {
    pub fn new(goal: impl Into<String>) -> Self {
        Self {
            goal: goal.into(),
            constraints: Vec::new(),
            target_files: Vec::new(),
            budget: None,
            priority: TaskPriority::Normal,
            labels: HashMap::new(),
        }
    }

    pub fn with_constraint(mut self, constraint: impl Into<String>) -> Self {
        self.constraints.push(constraint.into());
        self
    }

    pub fn with_file(mut self, file: impl Into<String>) -> Self {
        self.target_files.push(file.into());
        self
    }

    pub fn with_budget(mut self, budget: TaskBudgetSpec) -> Self {
        self.budget = Some(budget);
        self
    }

    pub fn with_priority(mut self, priority: TaskPriority) -> Self {
        self.priority = priority;
        self
    }

    pub fn with_label(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.labels.insert(key.into(), value.into());
        self
    }

    /// Validate this packet against the given limits.
    pub fn validate(
        &self,
        limits: &TaskPacketLimits,
    ) -> Result<ValidatedPacket, Vec<TaskPacketError>> {
        let mut errors = Vec::new();

        if self.goal.trim().is_empty() {
            errors.push(TaskPacketError::EmptyGoal);
        } else if self.goal.len() > limits.max_goal_len {
            errors.push(TaskPacketError::GoalTooLong {
                len: self.goal.len(),
                max: limits.max_goal_len,
            });
        }

        if self.constraints.len() > limits.max_constraints {
            errors.push(TaskPacketError::TooManyConstraints {
                count: self.constraints.len(),
                max: limits.max_constraints,
            });
        }

        if self.target_files.len() > limits.max_target_files {
            errors.push(TaskPacketError::TooManyFiles {
                count: self.target_files.len(),
                max: limits.max_target_files,
            });
        }

        if let Some(budget) = &self.budget {
            if budget.max_turns == Some(0) {
                errors.push(TaskPacketError::InvalidBudget {
                    reason: "max_turns cannot be 0".into(),
                });
            }
            if budget.max_tokens == Some(0) {
                errors.push(TaskPacketError::InvalidBudget {
                    reason: "max_tokens cannot be 0".into(),
                });
            }
        }

        if errors.is_empty() {
            Ok(ValidatedPacket {
                packet: self.clone(),
            })
        } else {
            Err(errors)
        }
    }
}

/// A task packet that has passed validation. Cannot be constructed directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedPacket {
    packet: TaskPacket,
}

impl ValidatedPacket {
    pub fn goal(&self) -> &str {
        &self.packet.goal
    }

    pub fn constraints(&self) -> &[String] {
        &self.packet.constraints
    }

    pub fn target_files(&self) -> &[String] {
        &self.packet.target_files
    }

    pub fn budget(&self) -> Option<&TaskBudgetSpec> {
        self.packet.budget.as_ref()
    }

    pub fn priority(&self) -> TaskPriority {
        self.packet.priority
    }

    pub fn labels(&self) -> &HashMap<String, String> {
        &self.packet.labels
    }

    pub fn into_packet(self) -> TaskPacket {
        self.packet
    }
}

/// Trait for custom task validators that can add domain-specific checks.
pub trait TaskValidator: Send + Sync + std::fmt::Debug {
    fn validate(&self, packet: &TaskPacket) -> Vec<TaskPacketError>;
    fn name(&self) -> &str;
}

/// Validates that all target files exist on disk.
#[derive(Debug)]
pub struct FileExistenceValidator;

impl TaskValidator for FileExistenceValidator {
    fn validate(&self, packet: &TaskPacket) -> Vec<TaskPacketError> {
        let mut errors = Vec::new();
        for file in &packet.target_files {
            if !std::path::Path::new(file).exists() {
                errors.push(TaskPacketError::InvalidBudget {
                    reason: format!("target file does not exist: {file}"),
                });
            }
        }
        errors
    }
    fn name(&self) -> &str {
        "file-existence"
    }
}

/// Validates that the budget is within acceptable ranges.
#[derive(Debug)]
pub struct BudgetRangeValidator {
    pub max_turns_limit: u32,
    pub max_tokens_limit: u64,
}

impl Default for BudgetRangeValidator {
    fn default() -> Self {
        Self {
            max_turns_limit: 100,
            max_tokens_limit: 1_000_000,
        }
    }
}

impl TaskValidator for BudgetRangeValidator {
    fn validate(&self, packet: &TaskPacket) -> Vec<TaskPacketError> {
        let mut errors = Vec::new();
        if let Some(budget) = &packet.budget {
            if let Some(turns) = budget.max_turns
                && turns > self.max_turns_limit
            {
                errors.push(TaskPacketError::InvalidBudget {
                    reason: format!("max_turns {turns} exceeds limit {}", self.max_turns_limit),
                });
            }
            if let Some(tokens) = budget.max_tokens
                && tokens > self.max_tokens_limit
            {
                errors.push(TaskPacketError::InvalidBudget {
                    reason: format!(
                        "max_tokens {tokens} exceeds limit {}",
                        self.max_tokens_limit
                    ),
                });
            }
        }
        errors
    }
    fn name(&self) -> &str {
        "budget-range"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_packet_passes() {
        let packet = TaskPacket::new("Fix the login bug")
            .with_constraint("Do not modify auth.rs")
            .with_file("src/login.rs")
            .with_priority(TaskPriority::High)
            .with_label("team", "backend");
        let validated = packet.validate(&TaskPacketLimits::default()).unwrap();
        assert_eq!(validated.goal(), "Fix the login bug");
        assert_eq!(validated.constraints().len(), 1);
        assert_eq!(validated.target_files().len(), 1);
        assert_eq!(validated.priority(), TaskPriority::High);
        assert_eq!(validated.labels().get("team").unwrap(), "backend");
    }

    #[test]
    fn empty_goal_rejected() {
        let packet = TaskPacket::new("");
        let errors = packet.validate(&TaskPacketLimits::default()).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, TaskPacketError::EmptyGoal))
        );
    }

    #[test]
    fn whitespace_goal_rejected() {
        let packet = TaskPacket::new("   ");
        let errors = packet.validate(&TaskPacketLimits::default()).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, TaskPacketError::EmptyGoal))
        );
    }

    #[test]
    fn goal_too_long_rejected() {
        let long_goal = "x".repeat(3000);
        let packet = TaskPacket::new(long_goal);
        let errors = packet.validate(&TaskPacketLimits::default()).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, TaskPacketError::GoalTooLong { .. }))
        );
    }

    #[test]
    fn too_many_constraints_rejected() {
        let mut packet = TaskPacket::new("goal");
        for i in 0..25 {
            packet.constraints.push(format!("constraint {i}"));
        }
        let errors = packet.validate(&TaskPacketLimits::default()).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, TaskPacketError::TooManyConstraints { .. }))
        );
    }

    #[test]
    fn too_many_files_rejected() {
        let mut packet = TaskPacket::new("goal");
        for i in 0..55 {
            packet.target_files.push(format!("file_{i}.rs"));
        }
        let errors = packet.validate(&TaskPacketLimits::default()).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, TaskPacketError::TooManyFiles { .. }))
        );
    }

    #[test]
    fn zero_budget_rejected() {
        let packet = TaskPacket::new("goal").with_budget(TaskBudgetSpec {
            max_turns: Some(0),
            max_tokens: None,
            max_tool_calls: None,
        });
        let errors = packet.validate(&TaskPacketLimits::default()).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, TaskPacketError::InvalidBudget { .. }))
        );
    }

    #[test]
    fn valid_budget_passes() {
        let packet = TaskPacket::new("goal").with_budget(TaskBudgetSpec {
            max_turns: Some(10),
            max_tokens: Some(50_000),
            max_tool_calls: Some(20),
        });
        let validated = packet.validate(&TaskPacketLimits::default()).unwrap();
        let budget = validated.budget().unwrap();
        assert_eq!(budget.max_turns, Some(10));
        assert_eq!(budget.max_tokens, Some(50_000));
    }

    #[test]
    fn priority_ordering() {
        assert!(TaskPriority::Low < TaskPriority::Normal);
        assert!(TaskPriority::Normal < TaskPriority::High);
        assert!(TaskPriority::High < TaskPriority::Critical);
    }

    #[test]
    fn validated_packet_into_packet() {
        let packet = TaskPacket::new("roundtrip");
        let validated = packet.validate(&TaskPacketLimits::default()).unwrap();
        let recovered = validated.into_packet();
        assert_eq!(recovered.goal, "roundtrip");
    }

    #[test]
    fn budget_range_validator_rejects_excessive() {
        let validator = BudgetRangeValidator::default();
        let packet = TaskPacket::new("goal").with_budget(TaskBudgetSpec {
            max_turns: Some(200),
            max_tokens: None,
            max_tool_calls: None,
        });
        let errors = validator.validate(&packet);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].to_string().contains("exceeds limit"));
    }

    #[test]
    fn budget_range_validator_passes_within_limits() {
        let validator = BudgetRangeValidator::default();
        let packet = TaskPacket::new("goal").with_budget(TaskBudgetSpec {
            max_turns: Some(50),
            max_tokens: Some(500_000),
            max_tool_calls: None,
        });
        assert!(validator.validate(&packet).is_empty());
    }

    #[test]
    fn multiple_errors_collected() {
        let mut packet = TaskPacket::new("");
        for i in 0..25 {
            packet.constraints.push(format!("c{i}"));
        }
        packet.budget = Some(TaskBudgetSpec {
            max_turns: Some(0),
            max_tokens: None,
            max_tool_calls: None,
        });
        let errors = packet.validate(&TaskPacketLimits::default()).unwrap_err();
        assert!(errors.len() >= 3); // EmptyGoal + TooManyConstraints + InvalidBudget
    }

    #[test]
    fn custom_limits() {
        let limits = TaskPacketLimits {
            max_goal_len: 10,
            max_constraints: 2,
            max_target_files: 1,
        };
        let packet = TaskPacket::new("short goal")
            .with_constraint("a")
            .with_constraint("b")
            .with_constraint("c");
        let errors = packet.validate(&limits).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, TaskPacketError::TooManyConstraints { .. }))
        );
    }

    #[test]
    fn error_display() {
        assert_eq!(
            TaskPacketError::EmptyGoal.to_string(),
            "task goal must not be empty"
        );
        let e = TaskPacketError::GoalTooLong {
            len: 3000,
            max: 2000,
        };
        assert!(e.to_string().contains("3000"));
    }
}
