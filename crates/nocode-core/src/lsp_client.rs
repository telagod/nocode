use std::collections::HashMap;
use std::path::Path;
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
        let result = match action {
            LspAction::Diagnostics => self.execute_diagnostics(file),
            LspAction::Hover => self.execute_hover(file, line, column),
            LspAction::Definition => self.execute_definition(file, line, column),
            LspAction::References => self.execute_references(file, line, column),
            LspAction::Completion => self.execute_completion(file, line),
            LspAction::Symbols => self.execute_symbols(file),
        };
        Ok(result)
    }

    fn execute_diagnostics(&self, file: &str) -> LspResult {
        let mut diags = Vec::new();
        let path = Path::new(file);

        if !path.exists() {
            diags.push(LspDiagnostic {
                file: file.to_string(),
                line: 0,
                column: 0,
                severity: "error".to_string(),
                message: format!("file not found: {file}"),
            });
            return LspResult::Diagnostics(diags);
        }

        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                diags.push(LspDiagnostic {
                    file: file.to_string(),
                    line: 0,
                    column: 0,
                    severity: "error".to_string(),
                    message: format!("cannot read file: {e}"),
                });
                return LspResult::Diagnostics(diags);
            }
        };

        let mut paren = 0i32;
        let mut brace = 0i32;
        let mut bracket = 0i32;
        for (i, line_str) in content.lines().enumerate() {
            for ch in line_str.chars() {
                match ch {
                    '(' => paren += 1,
                    ')' => paren -= 1,
                    '{' => brace += 1,
                    '}' => brace -= 1,
                    '[' => bracket += 1,
                    ']' => bracket -= 1,
                    _ => {}
                }
            }
            if let Some(col) = line_str.find("TODO") {
                diags.push(LspDiagnostic {
                    file: file.to_string(),
                    line: (i + 1) as u32,
                    column: col as u32,
                    severity: "warning".to_string(),
                    message: "TODO marker found".to_string(),
                });
            }
            if let Some(col) = line_str.find("FIXME") {
                diags.push(LspDiagnostic {
                    file: file.to_string(),
                    line: (i + 1) as u32,
                    column: col as u32,
                    severity: "warning".to_string(),
                    message: "FIXME marker found".to_string(),
                });
            }
            if line_str.len() > 120 {
                diags.push(LspDiagnostic {
                    file: file.to_string(),
                    line: (i + 1) as u32,
                    column: 120,
                    severity: "warning".to_string(),
                    message: format!("line exceeds 120 characters ({} chars)", line_str.len()),
                });
            }
        }

        let total_lines = content.lines().count() as u32;
        if paren != 0 {
            diags.push(LspDiagnostic {
                file: file.to_string(),
                line: total_lines,
                column: 0,
                severity: "error".to_string(),
                message: format!("unbalanced parentheses (balance: {paren})"),
            });
        }
        if brace != 0 {
            diags.push(LspDiagnostic {
                file: file.to_string(),
                line: total_lines,
                column: 0,
                severity: "error".to_string(),
                message: format!("unbalanced braces (balance: {brace})"),
            });
        }
        if bracket != 0 {
            diags.push(LspDiagnostic {
                file: file.to_string(),
                line: total_lines,
                column: 0,
                severity: "error".to_string(),
                message: format!("unbalanced brackets (balance: {bracket})"),
            });
        }

        LspResult::Diagnostics(diags)
    }

    fn execute_hover(&self, file: &str, line: u32, column: u32) -> LspResult {
        let content = match std::fs::read_to_string(file) {
            Ok(c) => c,
            Err(_) => return LspResult::Hover(None),
        };
        let lines_vec: Vec<&str> = content.lines().collect();
        let line_idx = line.saturating_sub(1) as usize;
        let target_line = match lines_vec.get(line_idx) {
            Some(l) => *l,
            None => return LspResult::Hover(None),
        };
        let token = match extract_symbol_at_position(target_line, column) {
            Some(t) => t,
            None => return LspResult::Hover(None),
        };

        // Search for definition in current file, then sibling files
        let mut def_locations = find_definitions_in_file(file, &token);
        if def_locations.is_empty()
            && let Some(parent) = Path::new(file).parent()
            && let Ok(entries) = std::fs::read_dir(parent)
        {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_file() && p.to_str() != Some(file) {
                    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
                    if matches!(ext, "rs" | "ts" | "js" | "py")
                        && let Some(ps) = p.to_str()
                    {
                        def_locations.extend(find_definitions_in_file(ps, &token));
                    }
                }
            }
        }

        // If we found a definition, extract doc comments above it
        let mut hover_content = String::new();
        if let Some(def_loc) = def_locations.first()
            && let Ok(def_content) = std::fs::read_to_string(&def_loc.file)
        {
            let def_lines: Vec<&str> = def_content.lines().collect();
            let def_line_idx = def_loc.line.saturating_sub(1) as usize;
            // Collect /// doc comments above the definition
            let mut doc_lines = Vec::new();
            let mut check_idx = def_line_idx;
            while check_idx > 0 {
                check_idx -= 1;
                let trimmed = def_lines.get(check_idx).map(|l| l.trim()).unwrap_or("");
                if let Some(doc) = trimmed.strip_prefix("///") {
                    doc_lines.push(doc.trim().to_string());
                } else {
                    break;
                }
            }
            doc_lines.reverse();
            if !doc_lines.is_empty() {
                hover_content.push_str(&doc_lines.join("\n"));
                hover_content.push('\n');
            }
            if let Some(def_line) = def_lines.get(def_line_idx) {
                hover_content.push_str(def_line.trim());
            }
        }

        if hover_content.is_empty() {
            hover_content = format!("token at {line}:{column}: {token}");
        }

        let col = column as usize;
        let bytes = target_line.as_bytes();
        let mut start = col;
        let mut end = col;
        while start > 0
            && ((bytes[start - 1] as char).is_alphanumeric() || bytes[start - 1] == b'_')
        {
            start -= 1;
        }
        while end < bytes.len() && ((bytes[end] as char).is_alphanumeric() || bytes[end] == b'_') {
            end += 1;
        }

        LspResult::Hover(Some(LspHoverResult {
            content: hover_content,
            range: Some((line, start as u32, line, end as u32)),
        }))
    }

    fn execute_definition(&self, file: &str, line: u32, column: u32) -> LspResult {
        let symbol = self.extract_symbol_at(file, line, column);
        let symbol = match symbol {
            Some(s) => s,
            None => return LspResult::Definition(vec![]),
        };
        // Search current file first
        let mut locations = find_definitions_in_file(file, &symbol);
        // Then search sibling files
        if let Some(parent) = Path::new(file).parent()
            && let Ok(entries) = std::fs::read_dir(parent)
        {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_file() && p.to_str() != Some(file) {
                    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
                    if matches!(ext, "rs" | "ts" | "js" | "py")
                        && let Some(ps) = p.to_str()
                    {
                        locations.extend(find_definitions_in_file(ps, &symbol));
                    }
                }
            }
        }
        LspResult::Definition(locations)
    }

    fn execute_references(&self, file: &str, line: u32, column: u32) -> LspResult {
        let symbol = self.extract_symbol_at(file, line, column);
        let symbol = match symbol {
            Some(s) => s,
            None => return LspResult::References(vec![]),
        };
        // Search current file first
        let mut locations = find_references_in_file(file, &symbol);
        // Then search sibling files
        if let Some(parent) = Path::new(file).parent()
            && let Ok(entries) = std::fs::read_dir(parent)
        {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_file() && p.to_str() != Some(file) {
                    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
                    if matches!(ext, "rs" | "ts" | "js" | "py")
                        && let Some(ps) = p.to_str()
                    {
                        locations.extend(find_references_in_file(ps, &symbol));
                    }
                }
            }
        }
        LspResult::References(locations)
    }

    fn execute_completion(&self, file: &str, line: u32) -> LspResult {
        let content = match std::fs::read_to_string(file) {
            Ok(c) => c,
            Err(_) => return LspResult::Completion(vec![]),
        };
        let target_line = content
            .lines()
            .nth((line.saturating_sub(1)) as usize)
            .unwrap_or("");
        let prefix = target_line.trim();
        let ext = Path::new(file)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        let keywords: &[&str] = match ext {
            "rs" => &[
                "fn", "let", "mut", "pub", "struct", "enum", "impl", "trait", "use", "mod",
                "match", "if", "else", "for", "while", "loop", "return", "self", "super", "crate",
                "const", "static", "async", "await", "where", "type",
            ],
            "ts" | "js" => &[
                "function",
                "const",
                "let",
                "var",
                "class",
                "interface",
                "import",
                "export",
                "async",
                "await",
                "return",
                "if",
                "else",
                "for",
                "while",
            ],
            "py" => &[
                "def", "class", "import", "from", "return", "if", "else", "elif", "for", "while",
                "with", "as", "try", "except", "async", "await", "yield",
            ],
            _ => &[],
        };
        let mut items: Vec<LspCompletionItem> = keywords
            .iter()
            .filter(|kw| kw.starts_with(prefix) && **kw != prefix)
            .map(|kw| LspCompletionItem {
                label: kw.to_string(),
                kind: "keyword".to_string(),
                detail: None,
            })
            .collect();

        // Extract symbols from the file as completion candidates
        let sym_patterns: &[(&str, &str)] = &[
            ("fn ", "function"),
            ("struct ", "struct"),
            ("enum ", "enum"),
            ("trait ", "trait"),
            ("const ", "constant"),
            ("type ", "type"),
        ];
        let mut seen = std::collections::HashSet::new();
        for kw in keywords {
            seen.insert(kw.to_string());
        }
        for line_str in content.lines() {
            let trimmed = line_str.trim();
            let stripped = if let Some(rest) = trimmed.strip_prefix("pub") {
                if rest.starts_with('(') {
                    match rest.find(')') {
                        Some(i) => rest[i + 1..].trim_start(),
                        None => rest.trim_start(),
                    }
                } else {
                    rest.trim_start()
                }
            } else {
                trimmed
            };
            for &(pat, kind) in sym_patterns {
                if let Some(after) = stripped.strip_prefix(pat) {
                    let name: String = after
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    if !name.is_empty()
                        && name.starts_with(prefix)
                        && name != prefix
                        && seen.insert(name.clone())
                    {
                        items.push(LspCompletionItem {
                            label: name,
                            kind: kind.to_string(),
                            detail: None,
                        });
                    }
                }
            }
        }

        LspResult::Completion(items)
    }

    fn execute_symbols(&self, file: &str) -> LspResult {
        let content = match std::fs::read_to_string(file) {
            Ok(c) => c,
            Err(_) => return LspResult::Symbols(vec![]),
        };
        let mut symbols = Vec::new();
        let patterns: &[(&str, &str)] = &[
            ("fn ", "function"),
            ("struct ", "struct"),
            ("enum ", "enum"),
            ("trait ", "trait"),
            ("impl ", "impl"),
            ("mod ", "module"),
            ("const ", "constant"),
            ("static ", "variable"),
            ("type ", "type"),
        ];
        for (line_num, line_str) in content.lines().enumerate() {
            let trimmed = line_str.trim();
            let stripped = if let Some(rest) = trimmed.strip_prefix("pub") {
                if rest.starts_with('(') {
                    match rest.find(')') {
                        Some(i) => rest[i + 1..].trim_start(),
                        None => rest.trim_start(),
                    }
                } else {
                    rest.trim_start()
                }
            } else {
                trimmed
            };
            for &(pat, kind) in patterns {
                if let Some(after) = stripped.strip_prefix(pat) {
                    let name: String = after
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    if !name.is_empty() {
                        let col = line_str.find(&name).unwrap_or(0);
                        symbols.push(LspSymbol {
                            name,
                            kind: kind.to_string(),
                            location: LspLocation {
                                file: file.to_string(),
                                line: (line_num + 1) as u32,
                                column: col as u32,
                            },
                        });
                    }
                    break;
                }
            }
        }
        LspResult::Symbols(symbols)
    }

    fn extract_symbol_at(&self, file: &str, line: u32, column: u32) -> Option<String> {
        let content = std::fs::read_to_string(file).ok()?;
        let target_line = content.lines().nth((line.saturating_sub(1)) as usize)?;
        extract_symbol_at_position(target_line, column)
    }
}

/// Extract the identifier at the given column position in a line of text.
/// Returns `None` if the column is out of bounds or not on an identifier character.
fn extract_symbol_at_position(line: &str, column: u32) -> Option<String> {
    let col = column as usize;
    let bytes = line.as_bytes();
    if col >= bytes.len() {
        return None;
    }
    if !(bytes[col] as char).is_alphanumeric() && bytes[col] != b'_' {
        return None;
    }
    let mut start = col;
    let mut end = col;
    while start > 0 && ((bytes[start - 1] as char).is_alphanumeric() || bytes[start - 1] == b'_') {
        start -= 1;
    }
    while end < bytes.len() && ((bytes[end] as char).is_alphanumeric() || bytes[end] == b'_') {
        end += 1;
    }
    let token = &line[start..end];
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

/// Search a file for definition patterns of the given symbol.
/// Looks for `fn symbol`, `struct symbol`, `enum symbol`, `trait symbol`,
/// `type symbol`, `const symbol`, `static symbol`.
fn find_definitions_in_file(file_path: &str, symbol: &str) -> Vec<LspLocation> {
    let content = match std::fs::read_to_string(file_path) {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    let def_patterns: &[&str] = &[
        "fn ", "struct ", "enum ", "trait ", "type ", "const ", "static ",
    ];
    let mut locations = Vec::new();
    for (i, line_str) in content.lines().enumerate() {
        let trimmed = line_str.trim();
        // Strip pub / pub(crate) / pub(super) prefix
        let stripped = if let Some(rest) = trimmed.strip_prefix("pub") {
            if rest.starts_with('(') {
                match rest.find(')') {
                    Some(idx) => rest[idx + 1..].trim_start(),
                    None => rest.trim_start(),
                }
            } else {
                rest.trim_start()
            }
        } else {
            trimmed
        };
        for pat in def_patterns {
            if let Some(after) = stripped.strip_prefix(pat) {
                let name: String = after
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if name == symbol {
                    let col = line_str.find(&name).unwrap_or(0);
                    locations.push(LspLocation {
                        file: file_path.to_string(),
                        line: (i + 1) as u32,
                        column: col as u32,
                    });
                }
            }
        }
    }
    locations
}

/// Search a file for all occurrences of the given symbol as a whole word.
fn find_references_in_file(file_path: &str, symbol: &str) -> Vec<LspLocation> {
    let content = match std::fs::read_to_string(file_path) {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    let mut locations = Vec::new();
    let sym_bytes = symbol.as_bytes();
    for (i, line_str) in content.lines().enumerate() {
        let bytes = line_str.as_bytes();
        let mut pos = 0;
        while pos + sym_bytes.len() <= bytes.len() {
            if let Some(idx) = line_str[pos..].find(symbol) {
                let abs_idx = pos + idx;
                // Check word boundaries
                let before_ok = abs_idx == 0
                    || (!(bytes[abs_idx - 1] as char).is_alphanumeric()
                        && bytes[abs_idx - 1] != b'_');
                let after_idx = abs_idx + sym_bytes.len();
                let after_ok = after_idx >= bytes.len()
                    || (!(bytes[after_idx] as char).is_alphanumeric() && bytes[after_idx] != b'_');
                if before_ok && after_ok {
                    locations.push(LspLocation {
                        file: file_path.to_string(),
                        line: (i + 1) as u32,
                        column: abs_idx as u32,
                    });
                }
                pos = abs_idx + sym_bytes.len();
            } else {
                break;
            }
        }
    }
    locations
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
    fn execute_diagnostics_on_missing_file() {
        let mut reg = LspRegistry::new();
        reg.register("ra", vec!["rust".into()]);
        reg.connect("ra").unwrap();
        let result = reg
            .execute("ra", LspAction::Diagnostics, "nonexistent_file.rs", 1, 0)
            .unwrap();
        match result {
            LspResult::Diagnostics(diags) => {
                assert_eq!(diags.len(), 1);
                assert_eq!(diags[0].severity, "error");
                assert!(diags[0].message.contains("file not found"));
            }
            _ => panic!("expected Diagnostics variant"),
        }
    }

    #[test]
    fn parse_lsp_action() {
        assert_eq!(
            LspAction::parse("diagnostics").unwrap(),
            LspAction::Diagnostics
        );
        assert_eq!(LspAction::parse("hover").unwrap(), LspAction::Hover);
        assert_eq!(
            LspAction::parse("definition").unwrap(),
            LspAction::Definition
        );
        assert_eq!(
            LspAction::parse("references").unwrap(),
            LspAction::References
        );
        assert_eq!(
            LspAction::parse("completion").unwrap(),
            LspAction::Completion
        );
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
        reg1.lock()
            .unwrap()
            .register("test-lsp", vec!["test".into()]);
        assert!(reg2.lock().unwrap().get("test-lsp").is_some());
    }

    #[test]
    fn symbols_finds_rust_definitions() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("sample.rs");
        std::fs::write(
            &file_path,
            "pub fn hello_world() {}\nstruct MyStruct {\n    field: u32,\n}\npub(crate) enum Color {\n    Red,\n    Blue,\n}\ntrait Drawable {\n    fn draw(&self);\n}\nconst MAX: u32 = 100;\n",
        )
        .unwrap();

        let mut reg = LspRegistry::new();
        reg.register("ra", vec!["rust".into()]);
        reg.connect("ra").unwrap();
        let file_str = file_path.to_str().unwrap();
        let result = reg
            .execute("ra", LspAction::Symbols, file_str, 0, 0)
            .unwrap();
        match result {
            LspResult::Symbols(syms) => {
                let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
                assert!(
                    names.contains(&"hello_world"),
                    "missing hello_world: {names:?}"
                );
                assert!(names.contains(&"MyStruct"), "missing MyStruct: {names:?}");
                assert!(names.contains(&"Color"), "missing Color: {names:?}");
                assert!(names.contains(&"Drawable"), "missing Drawable: {names:?}");
                assert!(names.contains(&"draw"), "missing draw: {names:?}");
                assert!(names.contains(&"MAX"), "missing MAX: {names:?}");

                let hw = syms.iter().find(|s| s.name == "hello_world").unwrap();
                assert_eq!(hw.kind, "function");
                assert_eq!(hw.location.line, 1);

                let ms = syms.iter().find(|s| s.name == "MyStruct").unwrap();
                assert_eq!(ms.kind, "struct");
                assert_eq!(ms.location.line, 2);
            }
            _ => panic!("expected Symbols variant"),
        }
    }

    #[test]
    fn diagnostics_detects_unbalanced_braces() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("bad.rs");
        std::fs::write(&file_path, "fn main() {\n").unwrap();

        let mut reg = LspRegistry::new();
        reg.register("ra", vec!["rust".into()]);
        reg.connect("ra").unwrap();
        let file_str = file_path.to_str().unwrap();
        let result = reg
            .execute("ra", LspAction::Diagnostics, file_str, 1, 0)
            .unwrap();
        match result {
            LspResult::Diagnostics(diags) => {
                assert!(
                    diags
                        .iter()
                        .any(|d| d.message.contains("unbalanced braces")),
                    "expected unbalanced braces diagnostic: {diags:?}"
                );
            }
            _ => panic!("expected Diagnostics variant"),
        }
    }

    #[test]
    fn diagnostics_detects_todo_markers() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("todo.rs");
        std::fs::write(&file_path, "fn main() {\n    // TODO fix this\n}\n").unwrap();

        let mut reg = LspRegistry::new();
        reg.register("ra", vec!["rust".into()]);
        reg.connect("ra").unwrap();
        let file_str = file_path.to_str().unwrap();
        let result = reg
            .execute("ra", LspAction::Diagnostics, file_str, 1, 0)
            .unwrap();
        match result {
            LspResult::Diagnostics(diags) => {
                assert!(
                    diags.iter().any(|d| d.message.contains("TODO")),
                    "expected TODO diagnostic: {diags:?}"
                );
            }
            _ => panic!("expected Diagnostics variant"),
        }
    }

    #[test]
    fn hover_extracts_token() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("hover.rs");
        std::fs::write(&file_path, "fn hello_world() {}\n").unwrap();

        let mut reg = LspRegistry::new();
        reg.register("ra", vec!["rust".into()]);
        reg.connect("ra").unwrap();
        let file_str = file_path.to_str().unwrap();
        let result = reg.execute("ra", LspAction::Hover, file_str, 1, 3).unwrap();
        match result {
            LspResult::Hover(Some(hover)) => {
                assert!(
                    hover.content.contains("hello_world"),
                    "expected hello_world in hover: {}",
                    hover.content
                );
            }
            _ => panic!("expected Hover(Some) variant"),
        }
    }

    #[test]
    fn diagnostics_detects_long_lines() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("long.rs");
        let long_line = format!("let x = \"{}\";", "a".repeat(130));
        let content = format!("fn main() {{\n{long_line}\n}}\n");
        std::fs::write(&file_path, &content).unwrap();

        let mut reg = LspRegistry::new();
        reg.register("ra", vec!["rust".into()]);
        reg.connect("ra").unwrap();
        let file_str = file_path.to_str().unwrap();
        let result = reg
            .execute("ra", LspAction::Diagnostics, file_str, 1, 0)
            .unwrap();
        match result {
            LspResult::Diagnostics(diags) => {
                assert!(
                    diags
                        .iter()
                        .any(|d| d.message.contains("exceeds 120 characters")),
                    "expected long line diagnostic: {diags:?}"
                );
                let long_diag = diags
                    .iter()
                    .find(|d| d.message.contains("exceeds 120"))
                    .unwrap();
                assert_eq!(long_diag.severity, "warning");
                assert_eq!(long_diag.line, 2);
            }
            _ => panic!("expected Diagnostics variant"),
        }
    }

    #[test]
    fn diagnostics_detects_unmatched_brackets() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("brackets.rs");
        std::fs::write(&file_path, "fn main() {\n    let v = vec![1, 2;\n}\n").unwrap();

        let mut reg = LspRegistry::new();
        reg.register("ra", vec!["rust".into()]);
        reg.connect("ra").unwrap();
        let file_str = file_path.to_str().unwrap();
        let result = reg
            .execute("ra", LspAction::Diagnostics, file_str, 1, 0)
            .unwrap();
        match result {
            LspResult::Diagnostics(diags) => {
                assert!(
                    diags
                        .iter()
                        .any(|d| d.message.contains("unbalanced brackets")),
                    "expected unbalanced brackets diagnostic: {diags:?}"
                );
            }
            _ => panic!("expected Diagnostics variant"),
        }
    }
    #[test]
    fn definition_finds_function() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("defn.rs");
        std::fs::write(
            &file_path,
            "fn helper() {}\n\nfn main() {\n    helper();\n}\n",
        )
        .unwrap();

        let mut reg = LspRegistry::new();
        reg.register("ra", vec!["rust".into()]);
        reg.connect("ra").unwrap();
        let file_str = file_path.to_str().unwrap();
        let result = reg
            .execute("ra", LspAction::Definition, file_str, 4, 4)
            .unwrap();
        match result {
            LspResult::Definition(locs) => {
                assert!(!locs.is_empty(), "expected at least one definition");
                assert!(
                    locs.iter().any(|l| l.line == 1),
                    "expected definition at line 1: {locs:?}"
                );
            }
            _ => panic!("expected Definition variant"),
        }
    }

    #[test]
    fn references_finds_occurrences() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("refs.rs");
        std::fs::write(
            &file_path,
            "fn greet() {}\n\nfn main() {\n    greet();\n    greet();\n}\n",
        )
        .unwrap();

        let mut reg = LspRegistry::new();
        reg.register("ra", vec!["rust".into()]);
        reg.connect("ra").unwrap();
        let file_str = file_path.to_str().unwrap();
        let result = reg
            .execute("ra", LspAction::References, file_str, 1, 3)
            .unwrap();
        match result {
            LspResult::References(locs) => {
                assert!(
                    locs.len() >= 3,
                    "expected at least 3 references (1 def + 2 calls): {locs:?}"
                );
            }
            _ => panic!("expected References variant"),
        }
    }

    #[test]
    fn completion_includes_keywords() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("comp.rs");
        std::fs::write(&file_path, "fn main() {\n    ma\n}\n").unwrap();

        let mut reg = LspRegistry::new();
        reg.register("ra", vec!["rust".into()]);
        reg.connect("ra").unwrap();
        let file_str = file_path.to_str().unwrap();
        let result = reg
            .execute("ra", LspAction::Completion, file_str, 2, 0)
            .unwrap();
        match result {
            LspResult::Completion(items) => {
                let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
                assert!(
                    labels.contains(&"match"),
                    "expected 'match' in completions: {labels:?}"
                );
            }
            _ => panic!("expected Completion variant"),
        }
    }

    #[test]
    fn completion_includes_file_symbols() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("sym_comp.rs");
        std::fs::write(
            &file_path,
            "struct MyWidget {}\nfn my_func() {}\n\nfn main() {\n    My\n}\n",
        )
        .unwrap();

        let mut reg = LspRegistry::new();
        reg.register("ra", vec!["rust".into()]);
        reg.connect("ra").unwrap();
        let file_str = file_path.to_str().unwrap();
        let result = reg
            .execute("ra", LspAction::Completion, file_str, 5, 0)
            .unwrap();
        match result {
            LspResult::Completion(items) => {
                let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
                assert!(
                    labels.contains(&"MyWidget"),
                    "expected 'MyWidget' in completions: {labels:?}"
                );
            }
            _ => panic!("expected Completion variant"),
        }
    }

    #[test]
    fn symbols_extracts_definitions() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("syms.rs");
        std::fs::write(
            &file_path,
            "pub fn alpha() {}\nstruct Beta {}\nenum Gamma {}\ntrait Delta {}\nmod omega {}\nconst PI: f64 = 3.14;\nstatic COUNT: u32 = 0;\ntype Alias = u32;\n",
        )
        .unwrap();

        let mut reg = LspRegistry::new();
        reg.register("ra", vec!["rust".into()]);
        reg.connect("ra").unwrap();
        let file_str = file_path.to_str().unwrap();
        let result = reg
            .execute("ra", LspAction::Symbols, file_str, 0, 0)
            .unwrap();
        match result {
            LspResult::Symbols(syms) => {
                let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
                assert!(names.contains(&"alpha"), "missing alpha: {names:?}");
                assert!(names.contains(&"Beta"), "missing Beta: {names:?}");
                assert!(names.contains(&"Gamma"), "missing Gamma: {names:?}");
                assert!(names.contains(&"Delta"), "missing Delta: {names:?}");
                assert!(names.contains(&"omega"), "missing omega: {names:?}");
                assert!(names.contains(&"PI"), "missing PI: {names:?}");
                assert!(names.contains(&"COUNT"), "missing COUNT: {names:?}");
                assert!(names.contains(&"Alias"), "missing Alias: {names:?}");
            }
            _ => panic!("expected Symbols variant"),
        }
    }

    #[test]
    fn extract_symbol_at_cursor() {
        assert_eq!(
            extract_symbol_at_position("fn hello_world() {}", 3),
            Some("hello_world".to_string())
        );
        assert_eq!(
            extract_symbol_at_position("fn hello_world() {}", 0),
            Some("fn".to_string())
        );
        assert_eq!(extract_symbol_at_position("fn hello_world() {}", 15), None);
        assert_eq!(extract_symbol_at_position("", 0), None);
        assert_eq!(extract_symbol_at_position("fn test()", 99), None);
        assert_eq!(
            extract_symbol_at_position("let my_var = 42;", 4),
            Some("my_var".to_string())
        );
    }

    #[test]
    fn hover_shows_doc_comment() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("doc_hover.rs");
        std::fs::write(
            &file_path,
            "/// This function greets the world.\n/// It is very friendly.\nfn greet() {}\n\nfn main() {\n    greet();\n}\n",
        )
        .unwrap();

        let mut reg = LspRegistry::new();
        reg.register("ra", vec!["rust".into()]);
        reg.connect("ra").unwrap();
        let file_str = file_path.to_str().unwrap();
        let result = reg.execute("ra", LspAction::Hover, file_str, 6, 4).unwrap();
        match result {
            LspResult::Hover(Some(hover)) => {
                assert!(
                    hover.content.contains("greets the world"),
                    "expected doc comment in hover: {}",
                    hover.content
                );
                assert!(
                    hover.content.contains("fn greet"),
                    "expected definition line in hover: {}",
                    hover.content
                );
            }
            _ => panic!("expected Hover(Some) variant"),
        }
    }
}
