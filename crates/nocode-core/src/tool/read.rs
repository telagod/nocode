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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_extension_extracts_correctly() {
        assert_eq!(file_extension("/tmp/foo.png"), Some("png".to_string()));
        assert_eq!(file_extension("/tmp/foo.JPG"), Some("jpg".to_string()));
        assert_eq!(file_extension("/tmp/foo"), None);
    }

    #[test]
    fn mime_types_correct() {
        assert_eq!(mime_for_extension("png"), "image/png");
        assert_eq!(mime_for_extension("jpg"), "image/jpeg");
        assert_eq!(mime_for_extension("gif"), "image/gif");
        assert_eq!(mime_for_extension("webp"), "image/webp");
    }

    #[test]
    fn read_text_file() {
        let path = "/tmp/nocode_read_test.txt";
        std::fs::write(path, "line1\nline2\nline3\n").unwrap();
        let tool = ReadTool;
        let result = tool.execute(&json!({"file_path": path}));
        assert!(!result.is_error);
        assert!(result.content.contains("line1"));
        assert!(result.content.contains("line3"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn read_image_returns_base64() {
        let path = "/tmp/nocode_test_img.png";
        // Write minimal PNG header
        std::fs::write(path, b"\x89PNG\r\n\x1a\n").unwrap();
        let tool = ReadTool;
        let result = tool.execute(&json!({"file_path": path}));
        assert!(!result.is_error);
        assert!(result.content.contains("image/png"));
        assert!(result.content.contains("base64"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn read_notebook_parses_cells() {
        let path = "/tmp/nocode_test_nb.ipynb";
        let nb = json!({
            "cells": [
                {"cell_type": "code", "source": ["print('hello')\n"]},
                {"cell_type": "markdown", "source": ["# Title\n"]}
            ],
            "metadata": {},
            "nbformat": 4
        });
        std::fs::write(path, serde_json::to_string(&nb).unwrap()).unwrap();
        let tool = ReadTool;
        let result = tool.execute(&json!({"file_path": path}));
        assert!(!result.is_error);
        assert!(result.content.contains("Cell 1 (code)"));
        assert!(result.content.contains("print('hello')"));
        assert!(result.content.contains("Cell 2 (markdown)"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn read_pdf_returns_error() {
        let tool = ReadTool;
        let result = tool.execute(&json!({"file_path": "/tmp/test.pdf"}));
        assert!(result.is_error);
        assert!(result.content.contains("PDF"));
    }

    #[test]
    fn read_nonexistent_returns_error() {
        let tool = ReadTool;
        let result = tool.execute(&json!({"file_path": "/tmp/nocode_nonexistent_xyz_99999"}));
        assert!(result.is_error);
    }

    #[test]
    fn read_with_offset_and_limit() {
        let path = "/tmp/nocode_read_offset_test.txt";
        let content: String = (0..20).map(|i| format!("line {i}\n")).collect();
        std::fs::write(path, &content).unwrap();
        let tool = ReadTool;
        let result = tool.execute(&json!({"file_path": path, "offset": 5, "limit": 3}));
        assert!(!result.is_error);
        assert!(result.content.contains("line 5"));
        assert!(result.content.contains("line 7"));
        assert!(!result.content.contains("line 8"));
        let _ = std::fs::remove_file(path);
    }
}
