use crate::tool::file_safety;
use crate::tool::{Tool, ToolOutput};
use serde_json::{Value, json};
use std::fs;

const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp"];
const NOTEBOOK_EXTENSION: &str = "ipynb";

fn file_extension(path: &str) -> Option<String> {
    std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
}

fn mime_for_extension(ext: &str) -> &'static str {
    match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
}

pub struct ReadTool;

impl Tool for ReadTool {
    fn name(&self) -> &str {
        "FileRead"
    }

    fn description(&self) -> &str {
        "Read a file from the filesystem. Returns the file contents with line numbers."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": { "type": "string", "description": "The absolute path to the file to read" },
                "offset": { "type": "number", "description": "The line number to start reading from. Only provide if the file is too large to read at once" },
                "limit": { "type": "number", "description": "The number of lines to read. Only provide if the file is too large to read at once." },
                "pages": { "type": "string", "description": "Page range for PDF files (e.g., \"1-5\", \"3\", \"10-20\"). Only applicable to PDF files. Maximum 20 pages per request." }
            },
            "required": ["file_path"]
        })
    }

    fn execute(&self, input: &Value) -> ToolOutput {
        let Some(path) = input["file_path"].as_str() else {
            return ToolOutput::error("Missing required parameter: file_path");
        };

        let ext = file_extension(path).unwrap_or_default();

        // PDF handling
        if ext == "pdf" {
            return ToolOutput::error(
                "PDF reading requires a PDF extraction library. Use an external tool to convert PDF to text first.",
            );
        }

        // Image handling — return base64
        if IMAGE_EXTENSIONS.contains(&ext.as_str()) {
            return read_image(path, &ext);
        }

        // Notebook handling — return cell contents
        if ext == NOTEBOOK_EXTENSION {
            return read_notebook(path);
        }

        // Size limit check (10 MB)
        if let Err(e) = file_safety::check_file_size(path) {
            return ToolOutput::error(e);
        }

        // Binary detection
        if file_safety::is_binary_file(path) {
            return ToolOutput::error(format!(
                "{path} appears to be a binary file and cannot be displayed as text"
            ));
        }

        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => return ToolOutput::error(format!("Failed to read {path}: {e}")),
        };

        let offset = input["offset"].as_u64().unwrap_or(0) as usize;
        let limit = input["limit"].as_u64().unwrap_or(2000) as usize;

        let lines: Vec<&str> = content.lines().collect();
        let end = (offset + limit).min(lines.len());
        let selected = &lines[offset.min(lines.len())..end];

        let numbered: String = selected
            .iter()
            .enumerate()
            .map(|(i, line)| format!("{}\t{line}", offset + i + 1))
            .collect::<Vec<_>>()
            .join("\n");

        ToolOutput::success(numbered)
    }
}

/// Read an image file and return base64-encoded content.
fn read_image(path: &str, ext: &str) -> ToolOutput {
    use base64::Engine;

    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) => return ToolOutput::error(format!("Failed to read image {path}: {e}")),
    };

    let mime = mime_for_extension(ext);
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    ToolOutput::success(
        json!({
            "type": "image",
            "file": {
                "base64": b64,
                "type": mime,
                "originalSize": bytes.len(),
            }
        })
        .to_string(),
    )
}

/// Read a Jupyter notebook and return cell contents.
fn read_notebook(path: &str) -> ToolOutput {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => return ToolOutput::error(format!("Failed to read notebook {path}: {e}")),
    };

    let notebook: Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(e) => return ToolOutput::error(format!("Invalid notebook JSON: {e}")),
    };

    let cells = match notebook.get("cells").and_then(Value::as_array) {
        Some(c) => c,
        None => return ToolOutput::error("Notebook has no cells array"),
    };

    let mut output = String::new();
    for (i, cell) in cells.iter().enumerate() {
        let cell_type = cell["cell_type"].as_str().unwrap_or("unknown");
        let source = cell["source"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("")
            })
            .or_else(|| cell["source"].as_str().map(String::from))
            .unwrap_or_default();

        output.push_str(&format!("--- Cell {} ({cell_type}) ---\n", i + 1));
        output.push_str(&source);
        if !source.ends_with('\n') {
            output.push('\n');
        }
        output.push('\n');
    }

    ToolOutput::success(output)
}
