//! Trust & Permission system — chainable policies for tool execution.

/// Trust decision for a tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustDecision {
    Allow,
    Deny,
    PromptUser,
}

/// Context for trust evaluation.
#[derive(Debug, Clone)]
pub struct TrustContext {
    pub tool_name: String,
    pub origin: String,
    pub labels: Vec<String>,
}

impl TrustContext {
    pub fn new(tool_name: &str, origin: &str) -> Self {
        Self {
            tool_name: tool_name.to_string(),
            origin: origin.to_string(),
            labels: Vec::new(),
        }
    }

    pub fn with_label(mut self, label: &str) -> Self {
        self.labels.push(label.to_string());
        self
    }
}

/// Trait for trust decision engines.
pub trait TrustResolver: Send + Sync {
    fn resolve(&self, ctx: &TrustContext) -> TrustDecision;
}

/// Always allows — for local/CTF/test environments.
pub struct AllowAllPolicy;
impl TrustResolver for AllowAllPolicy {
    fn resolve(&self, _ctx: &TrustContext) -> TrustDecision {
        TrustDecision::Allow
    }
}

/// Always requires user prompt.
pub struct PromptRequiredPolicy;
impl TrustResolver for PromptRequiredPolicy {
    fn resolve(&self, _ctx: &TrustContext) -> TrustDecision {
        TrustDecision::PromptUser
    }
}

/// Rule-based: allow/deny labels, else prompt.
pub struct RuleBasedPolicy {
    pub allow_labels: Vec<String>,
    pub deny_labels: Vec<String>,
}

impl RuleBasedPolicy {
    pub fn new(allow: Vec<String>, deny: Vec<String>) -> Self {
        Self {
            allow_labels: allow,
            deny_labels: deny,
        }
    }
}

impl TrustResolver for RuleBasedPolicy {
    fn resolve(&self, ctx: &TrustContext) -> TrustDecision {
        for label in &ctx.labels {
            if self.deny_labels.iter().any(|d| d == label) {
                return TrustDecision::Deny;
            }
        }
        for label in &ctx.labels {
            if self.allow_labels.iter().any(|a| a == label) {
                return TrustDecision::Allow;
            }
        }
        TrustDecision::PromptUser
    }
}

/// Chainable resolver — tries each policy in order, first non-PromptUser wins.
pub struct ChainedResolver {
    resolvers: Vec<Box<dyn TrustResolver>>,
}

impl ChainedResolver {
    pub fn new() -> Self {
        Self {
            resolvers: Vec::new(),
        }
    }

    pub fn push_resolver(mut self, resolver: Box<dyn TrustResolver>) -> Self {
        self.resolvers.push(resolver);
        self
    }
}

impl Default for ChainedResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl TrustResolver for ChainedResolver {
    fn resolve(&self, ctx: &TrustContext) -> TrustDecision {
        for r in &self.resolvers {
            let decision = r.resolve(ctx);
            if decision != TrustDecision::PromptUser {
                return decision;
            }
        }
        TrustDecision::PromptUser
    }
}

/// Permission enforcer — wraps a TrustResolver and applies it to tool calls.
pub struct PermissionEnforcer {
    resolver: Box<dyn TrustResolver>,
}

impl PermissionEnforcer {
    pub fn new(resolver: Box<dyn TrustResolver>) -> Self {
        Self { resolver }
    }

    pub fn allow_all() -> Self {
        Self::new(Box::new(AllowAllPolicy))
    }

    pub fn prompt_required() -> Self {
        Self::new(Box::new(PromptRequiredPolicy))
    }

    /// Check if a tool call is permitted.
    pub fn check(&self, tool_name: &str, origin: &str) -> TrustDecision {
        let ctx = TrustContext::new(tool_name, origin);
        self.resolver.resolve(&ctx)
    }

    /// Check with labels.
    pub fn check_with_labels(
        &self,
        tool_name: &str,
        origin: &str,
        labels: &[&str],
    ) -> TrustDecision {
        let mut ctx = TrustContext::new(tool_name, origin);
        for l in labels {
            ctx.labels.push(l.to_string());
        }
        self.resolver.resolve(&ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allow_all_policy() {
        let p = AllowAllPolicy;
        assert_eq!(
            p.resolve(&TrustContext::new("Bash", "user")),
            TrustDecision::Allow
        );
    }

    #[test]
    fn prompt_required_policy() {
        let p = PromptRequiredPolicy;
        assert_eq!(
            p.resolve(&TrustContext::new("Bash", "user")),
            TrustDecision::PromptUser
        );
    }

    #[test]
    fn rule_based_deny() {
        let p = RuleBasedPolicy::new(vec![], vec!["dangerous".to_string()]);
        let ctx = TrustContext::new("Bash", "user").with_label("dangerous");
        assert_eq!(p.resolve(&ctx), TrustDecision::Deny);
    }

    #[test]
    fn rule_based_allow() {
        let p = RuleBasedPolicy::new(vec!["safe".to_string()], vec![]);
        let ctx = TrustContext::new("Read", "user").with_label("safe");
        assert_eq!(p.resolve(&ctx), TrustDecision::Allow);
    }

    #[test]
    fn chained_resolver() {
        let chain = ChainedResolver::new()
            .push_resolver(Box::new(RuleBasedPolicy::new(
                vec!["safe".to_string()],
                vec!["blocked".to_string()],
            )))
            .push_resolver(Box::new(PromptRequiredPolicy));

        let safe_ctx = TrustContext::new("Read", "user").with_label("safe");
        assert_eq!(chain.resolve(&safe_ctx), TrustDecision::Allow);

        let blocked_ctx = TrustContext::new("Bash", "user").with_label("blocked");
        assert_eq!(chain.resolve(&blocked_ctx), TrustDecision::Deny);

        let unknown_ctx = TrustContext::new("Write", "user");
        assert_eq!(chain.resolve(&unknown_ctx), TrustDecision::PromptUser);
    }

    #[test]
    fn permission_enforcer() {
        let enforcer = PermissionEnforcer::allow_all();
        assert_eq!(enforcer.check("Bash", "user"), TrustDecision::Allow);

        let enforcer2 = PermissionEnforcer::prompt_required();
        assert_eq!(enforcer2.check("Bash", "user"), TrustDecision::PromptUser);
    }
}
