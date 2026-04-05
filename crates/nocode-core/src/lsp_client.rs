use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LspAction {
    Diagnostics,
    Hover,
    Definition,
    References,
    Completion,
    Symbols,
}

impl LspAction {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "diagnostics" => Ok(Self::Diagnostics),
            "hover" => Ok(Self::Hover),
            "definition" => Ok(Self::Definition),
            "references" => Ok(Self::References),
            "completion" => Ok(Self::Completion),
            "symbols" => Ok(Self::Symbols),
            other => Err(format!("unknown LSP action: {other}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LspServerStatus {
    Connected,
    Disconnected,
    Starting,
    Error,
}

#[derive(Debug, Clone)]
pub struct LspDiagnostic {
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub severity: String,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct LspHoverResult {
    pub content: String,
    /// (start_line, start_col, end_line, end_col)
    pub range: Option<(u32, u32, u32, u32)>,
}

#[derive(Debug, Clone)]
pub struct LspLocation {
    pub file: String,
    pub line: u32,
    pub column: u32,
}

#[derive(Debug, Clone)]
pub struct LspCompletionItem {
    pub label: String,
    pub kind: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LspSymbol {
    pub name: String,
    pub kind: String,
    pub location: LspLocation,
}

#[derive(Debug, Clone)]
pub enum LspResult {
    Diagnostics(Vec<LspDiagnostic>),
    Hover(Option<LspHoverResult>),
    Definition(Vec<LspLocation>),
    References(Vec<LspLocation>),
    Completion(Vec<LspCompletionItem>),
    Symbols(Vec<LspSymbol>),
}

#[derive(Debug)]
pub struct LspServer {
    pub name: String,
    pub status: LspServerStatus,
    pub languages: Vec<String>,
}

pub struct LspRegistry {
    servers: HashMap<String, LspServer>,
}

impl Default for LspRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl LspRegistry {
    pub fn new() -> Self {
        Self {
            servers: HashMap::new(),
        }
    }

    pub fn register(&mut self, name: &str, languages: Vec<String>) {
        self.servers.insert(
            name.to_string(),
            LspServer {
                name: name.to_string(),
                status: LspServerStatus::Disconnected,
                languages,
            },
        );
    }

    pub fn connect(&mut self, name: &str) -> Result<(), String> {
        let server = self
            .servers
            .get_mut(name)
            .ok_or_else(|| format!("LSP server not found: {name}"))?;
        server.status = LspServerStatus::Connected;
        Ok(())
    }

    pub fn disconnect(&mut self, name: &str) {
        if let Some(server) = self.servers.get_mut(name) {
            server.status = LspServerStatus::Disconnected;
        }
    }

    pub fn get(&self, name: &str) -> Option<&LspServer> {
        self.servers.get(name)
    }

    pub fn find_for_language(&self, lang: &str) -> Option<&LspServer> {
        self.servers.values().find(|server| {
            server.status == LspServerStatus::Connected
                && server.languages.iter().any(|l| l == lang)
        })
    }

    pub fn list(&self) -> Vec<&LspServer> {
        self.servers.values().collect()
    }

    pub fn execute(
        &self,
        server_name: &str,
        action: LspAction,
        file: &str,
        line: u32,
        column: u32,
    ) -> Result<LspResult, String> {
        let server = self
            .servers
            .get(server_name)
            .ok_or_else(|| format!("LSP server not found: {server_name}"))?;
        if server.status != LspServerStatus::Connected {
            return Err(format!(
                "LSP server {server_name} not connected for real execution"
            ));
        }
        // Return mock responses for now.
        let result = match action {
            LspAction::Diagnostics => LspResult::Diagnostics(vec![LspDiagnostic {
                file: file.to_string(),
                line,
                column,
                severity: "info".to_string(),
                message: format!("mock diagnostic for {file}:{line}:{column}"),
            }]),
            LspAction::Hover => LspResult::Hover(Some(LspHoverResult {
                content: format!("mock hover at {file}:{line}:{column}"),
                range: Some((line, column, line, column + 1)),
            })),
            LspAction::Definition => LspResult::Definition(vec![LspLocation {
                file: file.to_string(),
                line,
                column,
            }]),
            LspAction::References => LspResult::References(vec![LspLocation {
                file: file.to_string(),
                line,
                column,
            }]),
            LspAction::Completion => LspResult::Completion(vec![LspCompletionItem {
                label: "mock_completion".to_string(),
                kind: "function".to_string(),
                detail: Some(format!("mock completion at {file}:{line}:{column}")),
            }]),
            LspAction::Symbols => LspResult::Symbols(vec![LspSymbol {
                name: "mock_symbol".to_string(),
                kind: "function".to_string(),
                location: LspLocation {
                    file: file.to_string(),
                    line,
                    column,
                },
            }]),
        };
        Ok(result)
    }
}

static LSP_REGISTRY: OnceLock<Arc<Mutex<LspRegistry>>> = OnceLock::new();

pub fn global_lsp_registry() -> Arc<Mutex<LspRegistry>> {
    LSP_REGISTRY
        .get_or_init(|| Arc::new(Mutex::new(LspRegistry::new())))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_connect_server() {
        let mut reg = LspRegistry::new();
        reg.register("rust-analyzer", vec!["rust".into()]);
        assert_eq!(
            reg.get("rust-analyzer").unwrap().status,
            LspServerStatus::Disconnected
        );
        reg.connect("rust-analyzer").unwrap();
        assert_eq!(
            reg.get("rust-analyzer").unwrap().status,
            LspServerStatus::Connected
        );
    }

    #[test]
    fn find_for_language_returns_connected() {
        let mut reg = LspRegistry::new();
        reg.register("rust-analyzer", vec!["rust".into()]);
        reg.connect("rust-analyzer").unwrap();
        let server = reg.find_for_language("rust").unwrap();
        assert_eq!(server.name, "rust-analyzer");
    }

    #[test]
    fn find_for_language_skips_disconnected() {
        let mut reg = LspRegistry::new();
        reg.register("rust-analyzer", vec!["rust".into()]);
        assert!(reg.find_for_language("rust").is_none());
    }

    #[test]
    fn execute_returns_mock_result() {
        let mut reg = LspRegistry::new();
        reg.register("ra", vec!["rust".into()]);
        reg.connect("ra").unwrap();
        let result = reg
            .execute("ra", LspAction::Diagnostics, "main.rs", 1, 0)
            .unwrap();
        match result {
            LspResult::Diagnostics(diags) => {
                assert_eq!(diags.len(), 1);
                assert_eq!(diags[0].file, "main.rs");
            }
            _ => panic!("expected Diagnostics variant"),
        }
    }

    #[test]
    fn parse_lsp_action() {
        assert_eq!(LspAction::parse("diagnostics").unwrap(), LspAction::Diagnostics);
        assert_eq!(LspAction::parse("hover").unwrap(), LspAction::Hover);
        assert_eq!(LspAction::parse("definition").unwrap(), LspAction::Definition);
        assert_eq!(LspAction::parse("references").unwrap(), LspAction::References);
        assert_eq!(LspAction::parse("completion").unwrap(), LspAction::Completion);
        assert_eq!(LspAction::parse("symbols").unwrap(), LspAction::Symbols);
        assert!(LspAction::parse("unknown").is_err());
    }

    #[test]
    fn execute_disconnected_server_fails() {
        let mut reg = LspRegistry::new();
        reg.register("ra", vec!["rust".into()]);
        let result = reg.execute("ra", LspAction::Hover, "main.rs", 1, 0);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not connected"));
    }

    #[test]
    fn global_singleton_works() {
        let reg1 = global_lsp_registry();
        let reg2 = global_lsp_registry();
        reg1.lock().unwrap().register("test-lsp", vec!["test".into()]);
        assert!(reg2.lock().unwrap().get("test-lsp").is_some());
    }
}
