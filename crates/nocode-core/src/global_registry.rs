use std::sync::{Arc, Mutex, OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolSource {
    /// Built-in tools shipped with the binary.
    Base,
    /// Tools injected by plugins.
    Plugin,
    /// Tools discovered via MCP.
    Mcp,
    /// Tools registered at runtime.
    Runtime,
}

#[derive(Debug, Clone)]
pub struct RegisteredTool {
    pub name: String,
    pub source: ToolSource,
    pub description: String,
    pub schema: serde_json::Value,
}

pub struct GlobalToolRegistry {
    tools: Vec<RegisteredTool>,
}

impl GlobalToolRegistry {
    pub fn new() -> Self {
        Self { tools: Vec::new() }
    }

    /// Register a tool. If a tool with the same name already exists, it is replaced.
    pub fn register(&mut self, tool: RegisteredTool) {
        if let Some(existing) = self.tools.iter_mut().find(|t| t.name == tool.name) {
            *existing = tool;
        } else {
            self.tools.push(tool);
        }
    }

    /// Remove a tool by name. Returns `true` if the tool was found and removed.
    pub fn unregister(&mut self, name: &str) -> bool {
        let len_before = self.tools.len();
        self.tools.retain(|t| t.name != name);
        self.tools.len() < len_before
    }

    pub fn get(&self, name: &str) -> Option<&RegisteredTool> {
        self.tools.iter().find(|t| t.name == name)
    }

    pub fn list(&self) -> &[RegisteredTool] {
        &self.tools
    }

    pub fn list_by_source(&self, source: ToolSource) -> Vec<&RegisteredTool> {
        self.tools.iter().filter(|t| t.source == source).collect()
    }

    /// Search tools by name or description (case-insensitive substring match).
    pub fn search(&self, query: &str) -> Vec<&RegisteredTool> {
        let q = query.to_lowercase();
        self.tools
            .iter()
            .filter(|t| {
                t.name.to_lowercase().contains(&q) || t.description.to_lowercase().contains(&q)
            })
            .collect()
    }

    pub fn count(&self) -> usize {
        self.tools.len()
    }
}

impl Default for GlobalToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Global singleton
// ---------------------------------------------------------------------------

static GLOBAL_REGISTRY: OnceLock<Arc<Mutex<GlobalToolRegistry>>> = OnceLock::new();

pub fn global_tool_registry() -> Arc<Mutex<GlobalToolRegistry>> {
    GLOBAL_REGISTRY
        .get_or_init(|| Arc::new(Mutex::new(GlobalToolRegistry::new())))
        .clone()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tool(name: &str, source: ToolSource, desc: &str) -> RegisteredTool {
        RegisteredTool {
            name: name.to_string(),
            source,
            description: desc.to_string(),
            schema: serde_json::json!({}),
        }
    }

    #[test]
    fn register_and_get() {
        let mut reg = GlobalToolRegistry::new();
        reg.register(make_tool("read_file", ToolSource::Base, "Read a file"));
        let tool = reg.get("read_file").expect("tool should exist");
        assert_eq!(tool.name, "read_file");
        assert_eq!(tool.source, ToolSource::Base);
        assert_eq!(tool.description, "Read a file");
    }

    #[test]
    fn register_overwrites_same_name() {
        let mut reg = GlobalToolRegistry::new();
        reg.register(make_tool("rw", ToolSource::Base, "v1"));
        reg.register(make_tool("rw", ToolSource::Plugin, "v2"));
        assert_eq!(reg.count(), 1);
        let tool = reg.get("rw").unwrap();
        assert_eq!(tool.source, ToolSource::Plugin);
        assert_eq!(tool.description, "v2");
    }

    #[test]
    fn unregister_removes_tool() {
        let mut reg = GlobalToolRegistry::new();
        reg.register(make_tool("tmp", ToolSource::Runtime, "temp"));
        assert!(reg.unregister("tmp"));
        assert!(reg.get("tmp").is_none());
        assert!(!reg.unregister("tmp"));
    }

    #[test]
    fn list_by_source_filters() {
        let mut reg = GlobalToolRegistry::new();
        reg.register(make_tool("a", ToolSource::Base, ""));
        reg.register(make_tool("b", ToolSource::Mcp, ""));
        reg.register(make_tool("c", ToolSource::Base, ""));
        let base = reg.list_by_source(ToolSource::Base);
        assert_eq!(base.len(), 2);
        assert!(base.iter().all(|t| t.source == ToolSource::Base));
    }

    #[test]
    fn search_matches_name_and_description() {
        let mut reg = GlobalToolRegistry::new();
        reg.register(make_tool("file_reader", ToolSource::Base, "Reads files"));
        reg.register(make_tool("bash", ToolSource::Base, "Execute shell"));
        reg.register(make_tool("grep", ToolSource::Base, "Search file contents"));

        let by_name = reg.search("file");
        assert_eq!(by_name.len(), 2); // file_reader + grep (description has "file")

        let by_desc = reg.search("SHELL");
        assert_eq!(by_desc.len(), 1);
        assert_eq!(by_desc[0].name, "bash");
    }

    #[test]
    fn global_singleton_is_same_instance() {
        let a = global_tool_registry();
        let b = global_tool_registry();
        assert!(Arc::ptr_eq(&a, &b));
    }
}
