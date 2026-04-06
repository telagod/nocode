use std::sync::{Arc, Mutex, OnceLock};

use crate::tool_registry::PermissionMode;

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginKind {
    Builtin,
    Bundled,
    External,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginState {
    Unconfigured,
    Validated,
    Starting,
    Healthy,
    Degraded,
    Failed,
    ShuttingDown,
    Stopped,
}

#[derive(Debug, Clone)]
pub struct PluginMetadata {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub kind: PluginKind,
    pub default_enabled: bool,
}

#[derive(Debug, Clone)]
pub struct PluginToolManifest {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub required_permission: PermissionMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEvent {
    PreToolUse,
    PostToolUse,
    PostToolUseFailure,
}

#[derive(Debug, Clone)]
pub struct PluginHookResult {
    pub event: HookEvent,
    pub plugin_id: String,
    pub denied: bool,
    pub message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Plugin {
    pub metadata: PluginMetadata,
    pub state: PluginState,
    pub tools: Vec<PluginToolManifest>,
    pub hooks: Vec<HookEvent>,
}

// ---------------------------------------------------------------------------
// PluginRegistry
// ---------------------------------------------------------------------------

pub struct PluginRegistry {
    plugins: Vec<Plugin>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
        }
    }

    /// Register a plugin. Validates metadata and transitions to `Validated`.
    pub fn register(&mut self, mut plugin: Plugin) -> Result<(), String> {
        if plugin.metadata.id.is_empty() {
            return Err("plugin id must not be empty".into());
        }
        if plugin.metadata.name.is_empty() {
            return Err("plugin name must not be empty".into());
        }
        if plugin.metadata.version.is_empty() {
            return Err("plugin version must not be empty".into());
        }
        if self
            .plugins
            .iter()
            .any(|p| p.metadata.id == plugin.metadata.id)
        {
            return Err(format!(
                "plugin '{}' is already registered",
                plugin.metadata.id
            ));
        }
        plugin.state = PluginState::Validated;
        self.plugins.push(plugin);
        Ok(())
    }

    /// Start a plugin: Validated -> Starting -> Healthy.
    pub fn start(&mut self, plugin_id: &str) -> Result<(), String> {
        let plugin = self
            .find_mut(plugin_id)
            .ok_or_else(|| format!("plugin '{plugin_id}' not found"))?;
        match plugin.state {
            PluginState::Validated | PluginState::Stopped => {
                plugin.state = PluginState::Starting;
                plugin.state = PluginState::Healthy;
                Ok(())
            }
            other => Err(format!(
                "cannot start plugin '{plugin_id}' from state {other:?}"
            )),
        }
    }

    /// Stop a plugin: * -> ShuttingDown -> Stopped.
    pub fn stop(&mut self, plugin_id: &str) -> Result<(), String> {
        let plugin = self
            .find_mut(plugin_id)
            .ok_or_else(|| format!("plugin '{plugin_id}' not found"))?;
        match plugin.state {
            PluginState::Healthy | PluginState::Degraded | PluginState::Starting => {
                plugin.state = PluginState::ShuttingDown;
                plugin.state = PluginState::Stopped;
                Ok(())
            }
            other => Err(format!(
                "cannot stop plugin '{plugin_id}' from state {other:?}"
            )),
        }
    }

    /// Mark a plugin as failed.
    pub fn fail(&mut self, plugin_id: &str, _reason: &str) {
        if let Some(plugin) = self.find_mut(plugin_id) {
            plugin.state = PluginState::Failed;
        }
    }

    /// Mark a plugin as degraded.
    pub fn degrade(&mut self, plugin_id: &str, _reason: &str) {
        if let Some(plugin) = self.find_mut(plugin_id) {
            plugin.state = PluginState::Degraded;
        }
    }

    pub fn get(&self, plugin_id: &str) -> Option<&Plugin> {
        self.plugins.iter().find(|p| p.metadata.id == plugin_id)
    }

    pub fn list(&self) -> &[Plugin] {
        &self.plugins
    }

    pub fn list_healthy(&self) -> Vec<&Plugin> {
        self.plugins
            .iter()
            .filter(|p| p.state == PluginState::Healthy)
            .collect()
    }

    pub fn tools_for(&self, plugin_id: &str) -> Vec<&PluginToolManifest> {
        self.get(plugin_id)
            .map(|p| p.tools.iter().collect())
            .unwrap_or_default()
    }

    /// Run a hook event across all healthy plugins that registered for it.
    pub fn run_hook(&self, event: HookEvent, _tool_name: &str) -> Vec<PluginHookResult> {
        self.plugins
            .iter()
            .filter(|p| p.state == PluginState::Healthy)
            .filter(|p| p.hooks.contains(&event))
            .map(|p| PluginHookResult {
                event,
                plugin_id: p.metadata.id.clone(),
                denied: false,
                message: None,
            })
            .collect()
    }

    fn find_mut(&mut self, plugin_id: &str) -> Option<&mut Plugin> {
        self.plugins.iter_mut().find(|p| p.metadata.id == plugin_id)
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Global singleton
// ---------------------------------------------------------------------------

static PLUGIN_REGISTRY: OnceLock<Arc<Mutex<PluginRegistry>>> = OnceLock::new();

pub fn global_plugin_registry() -> Arc<Mutex<PluginRegistry>> {
    PLUGIN_REGISTRY
        .get_or_init(|| Arc::new(Mutex::new(PluginRegistry::new())))
        .clone()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_metadata(id: &str) -> PluginMetadata {
        PluginMetadata {
            id: id.into(),
            name: format!("{id}-plugin"),
            version: "0.1.0".into(),
            description: format!("Test plugin {id}"),
            kind: PluginKind::External,
            default_enabled: true,
        }
    }

    fn make_plugin(id: &str) -> Plugin {
        Plugin {
            metadata: make_metadata(id),
            state: PluginState::Unconfigured,
            tools: vec![],
            hooks: vec![],
        }
    }

    fn make_plugin_with_tools(id: &str) -> Plugin {
        Plugin {
            metadata: make_metadata(id),
            state: PluginState::Unconfigured,
            tools: vec![
                PluginToolManifest {
                    name: format!("{id}_read"),
                    description: "read tool".into(),
                    input_schema: serde_json::json!({}),
                    required_permission: PermissionMode::ReadOnly,
                },
                PluginToolManifest {
                    name: format!("{id}_write"),
                    description: "write tool".into(),
                    input_schema: serde_json::json!({}),
                    required_permission: PermissionMode::WorkspaceWrite,
                },
            ],
            hooks: vec![HookEvent::PreToolUse],
        }
    }

    #[test]
    fn register_and_start_plugin() {
        let mut reg = PluginRegistry::new();
        reg.register(make_plugin("alpha")).unwrap();
        assert_eq!(reg.get("alpha").unwrap().state, PluginState::Validated);
        reg.start("alpha").unwrap();
        assert_eq!(reg.get("alpha").unwrap().state, PluginState::Healthy);
    }

    #[test]
    fn plugin_lifecycle_transitions() {
        let mut reg = PluginRegistry::new();
        reg.register(make_plugin("lc")).unwrap();
        assert_eq!(reg.get("lc").unwrap().state, PluginState::Validated);

        reg.start("lc").unwrap();
        assert_eq!(reg.get("lc").unwrap().state, PluginState::Healthy);

        reg.degrade("lc", "high latency");
        assert_eq!(reg.get("lc").unwrap().state, PluginState::Degraded);

        reg.stop("lc").unwrap();
        assert_eq!(reg.get("lc").unwrap().state, PluginState::Stopped);
    }

    #[test]
    fn invalid_transition_fails() {
        let mut reg = PluginRegistry::new();
        reg.register(make_plugin("bad")).unwrap();
        reg.start("bad").unwrap();
        reg.fail("bad", "crash");
        assert_eq!(reg.get("bad").unwrap().state, PluginState::Failed);

        let err = reg.start("bad").unwrap_err();
        assert!(err.contains("cannot start"));
    }

    #[test]
    fn list_healthy_filters_correctly() {
        let mut reg = PluginRegistry::new();
        reg.register(make_plugin("h1")).unwrap();
        reg.register(make_plugin("h2")).unwrap();
        reg.register(make_plugin("h3")).unwrap();
        reg.start("h1").unwrap();
        reg.start("h2").unwrap();
        // h3 stays Validated
        reg.fail("h2", "oops");

        let healthy = reg.list_healthy();
        assert_eq!(healthy.len(), 1);
        assert_eq!(healthy[0].metadata.id, "h1");
    }

    #[test]
    fn tools_for_returns_plugin_tools() {
        let mut reg = PluginRegistry::new();
        reg.register(make_plugin_with_tools("tp")).unwrap();
        let tools = reg.tools_for("tp");
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name, "tp_read");
        assert_eq!(tools[1].name, "tp_write");

        // Non-existent plugin returns empty
        assert!(reg.tools_for("nope").is_empty());
    }

    #[test]
    fn run_hook_collects_results() {
        let mut reg = PluginRegistry::new();

        let mut p1 = make_plugin("hook1");
        p1.hooks = vec![HookEvent::PreToolUse, HookEvent::PostToolUse];
        reg.register(p1).unwrap();
        reg.start("hook1").unwrap();

        let mut p2 = make_plugin("hook2");
        p2.hooks = vec![HookEvent::PreToolUse];
        reg.register(p2).unwrap();
        reg.start("hook2").unwrap();

        let mut p3 = make_plugin("hook3");
        p3.hooks = vec![HookEvent::PostToolUseFailure];
        reg.register(p3).unwrap();
        reg.start("hook3").unwrap();

        let pre = reg.run_hook(HookEvent::PreToolUse, "Bash");
        assert_eq!(pre.len(), 2);
        assert_eq!(pre[0].plugin_id, "hook1");
        assert_eq!(pre[1].plugin_id, "hook2");

        let post = reg.run_hook(HookEvent::PostToolUse, "Bash");
        assert_eq!(post.len(), 1);
        assert_eq!(post[0].plugin_id, "hook1");

        let fail = reg.run_hook(HookEvent::PostToolUseFailure, "Bash");
        assert_eq!(fail.len(), 1);
        assert_eq!(fail[0].plugin_id, "hook3");
    }

    #[test]
    fn global_singleton_works() {
        let a = global_plugin_registry();
        let b = global_plugin_registry();
        assert!(Arc::ptr_eq(&a, &b));
    }

    #[test]
    fn stop_and_restart_plugin() {
        let mut reg = PluginRegistry::new();
        reg.register(make_plugin("restart")).unwrap();
        reg.start("restart").unwrap();
        assert_eq!(reg.get("restart").unwrap().state, PluginState::Healthy);

        reg.stop("restart").unwrap();
        assert_eq!(reg.get("restart").unwrap().state, PluginState::Stopped);

        reg.start("restart").unwrap();
        assert_eq!(reg.get("restart").unwrap().state, PluginState::Healthy);
    }

    #[test]
    fn register_rejects_empty_id() {
        let mut reg = PluginRegistry::new();
        let mut p = make_plugin("x");
        p.metadata.id = String::new();
        assert!(
            reg.register(p)
                .unwrap_err()
                .contains("id must not be empty")
        );
    }

    #[test]
    fn register_rejects_duplicate() {
        let mut reg = PluginRegistry::new();
        reg.register(make_plugin("dup")).unwrap();
        let err = reg.register(make_plugin("dup")).unwrap_err();
        assert!(err.contains("already registered"));
    }

    #[test]
    fn list_returns_all() {
        let mut reg = PluginRegistry::new();
        reg.register(make_plugin("a")).unwrap();
        reg.register(make_plugin("b")).unwrap();
        assert_eq!(reg.list().len(), 2);
    }

    #[test]
    fn run_hook_skips_non_healthy() {
        let mut reg = PluginRegistry::new();
        let mut p = make_plugin("sick");
        p.hooks = vec![HookEvent::PreToolUse];
        reg.register(p).unwrap();
        // Validated, not Healthy — should not fire
        let results = reg.run_hook(HookEvent::PreToolUse, "Read");
        assert!(results.is_empty());
    }
}
