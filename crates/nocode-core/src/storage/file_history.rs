//! File history — undo/redo for file modifications made by nocode tools.
//!
//! Each modification is recorded as a snapshot (path + old content).
//! Undo restores the previous content and pushes the current to redo stack.
//! Redo restores the next content and pushes the current to undo stack.

use std::fs;
use std::path::{Path, PathBuf};

/// A single file edit record for undo/redo.
#[derive(Debug, Clone)]
pub struct FileEditRecord {
    /// File path that was modified.
    pub path: PathBuf,
    /// Content before the edit (None if file was created).
    pub old_content: Option<String>,
    /// Content after the edit (None if file was deleted).
    pub new_content: Option<String>,
}

/// Manages undo/redo stacks for file modifications.
pub struct FileHistory {
    undo_stack: Vec<FileEditRecord>,
    redo_stack: Vec<FileEditRecord>,
    #[allow(dead_code)]
    base_dir: PathBuf,
}

impl FileHistory {
    /// Create a new FileHistory rooted at the given directory.
    pub fn new(base_dir: &str) -> Result<Self, String> {
        Ok(Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            base_dir: PathBuf::from(base_dir),
        })
    }

    /// Record a file modification for potential undo.
    /// Call this *before* the modification with the old content.
    pub fn record_edit(
        &mut self,
        path: &Path,
        old_content: Option<String>,
        new_content: Option<String>,
    ) {
        self.undo_stack.push(FileEditRecord {
            path: path.to_path_buf(),
            old_content,
            new_content,
        });
        // Any new edit clears the redo stack
        self.redo_stack.clear();
    }

    /// Undo the most recent file modification.
    /// Returns the path of the restored file.
    pub fn undo(&mut self) -> Result<String, String> {
        let record = self.undo_stack.pop().ok_or("No edits to undo")?;

        // Restore old content
        match &record.old_content {
            Some(content) => {
                if let Err(e) = fs::write(&record.path, content) {
                    return Err(format!("Failed to restore {}: {e}", record.path.display()));
                }
            }
            None => {
                // File was created — delete it
                if record.path.exists()
                    && let Err(e) = fs::remove_file(&record.path)
                {
                    return Err(format!("Failed to delete {}: {e}", record.path.display()));
                }
            }
        }

        // Push to redo stack
        self.redo_stack.push(record.clone());

        Ok(format!("{}", record.path.display()))
    }

    /// Redo the most recently undone modification.
    /// Returns the path of the re-modified file.
    pub fn redo(&mut self) -> Result<String, String> {
        let record = self.redo_stack.pop().ok_or("No edits to redo")?;

        // Re-apply the new content
        match &record.new_content {
            Some(content) => {
                if let Some(parent) = record.path.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                if let Err(e) = fs::write(&record.path, content) {
                    return Err(format!("Failed to re-write {}: {e}", record.path.display()));
                }
            }
            None => {
                // File was deleted — delete it again
                if record.path.exists()
                    && let Err(e) = fs::remove_file(&record.path)
                {
                    return Err(format!("Failed to delete {}: {e}", record.path.display()));
                }
            }
        }

        // Push back to undo stack
        self.undo_stack.push(record.clone());

        Ok(format!("{}", record.path.display()))
    }

    /// Check if there are edits available to undo.
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// Check if there are edits available to redo.
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Number of edits in the undo stack.
    pub fn undo_count(&self) -> usize {
        self.undo_stack.len()
    }

    /// Number of edits in the redo stack.
    pub fn redo_count(&self) -> usize {
        self.redo_stack.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn undo_restores_old_content() {
        let dir = std::env::temp_dir().join("nocode_test_undo");
        let _ = fs::create_dir_all(&dir);
        let file_path = dir.join("test.txt");

        fs::write(&file_path, "original").unwrap();

        let mut history = FileHistory::new(dir.to_str().unwrap()).unwrap();
        history.record_edit(
            &file_path,
            Some("original".to_string()),
            Some("modified".to_string()),
        );

        // Simulate the edit
        fs::write(&file_path, "modified").unwrap();

        // Undo
        let result = history.undo();
        assert!(result.is_ok());

        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "original");

        // Cleanup
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn redo_reapplies_new_content() {
        let dir = std::env::temp_dir().join("nocode_test_redo");
        let _ = fs::create_dir_all(&dir);
        let file_path = dir.join("test.txt");

        fs::write(&file_path, "original").unwrap();

        let mut history = FileHistory::new(dir.to_str().unwrap()).unwrap();
        history.record_edit(
            &file_path,
            Some("original".to_string()),
            Some("modified".to_string()),
        );
        fs::write(&file_path, "modified").unwrap();

        history.undo().unwrap();
        let result = history.redo();
        assert!(result.is_ok());

        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "modified");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn undo_delete_restores_file() {
        let dir = std::env::temp_dir().join("nocode_test_undo_del");
        let _ = fs::create_dir_all(&dir);
        let file_path = dir.join("to_delete.txt");

        fs::write(&file_path, "content").unwrap();

        let mut history = FileHistory::new(dir.to_str().unwrap()).unwrap();
        history.record_edit(&file_path, Some("content".to_string()), None);

        fs::remove_file(&file_path).unwrap();

        history.undo().unwrap();
        assert!(file_path.exists());
        assert_eq!(fs::read_to_string(&file_path).unwrap(), "content");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn undo_create_deletes_file() {
        let dir = std::env::temp_dir().join("nocode_test_undo_create");
        let _ = fs::create_dir_all(&dir);
        let file_path = dir.join("new_file.txt");

        let mut history = FileHistory::new(dir.to_str().unwrap()).unwrap();
        history.record_edit(&file_path, None, Some("new content".to_string()));

        fs::write(&file_path, "new content").unwrap();

        history.undo().unwrap();
        assert!(!file_path.exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn new_edit_clears_redo_stack() {
        let dir = std::env::temp_dir().join("nocode_test_clear_redo");
        let _ = fs::create_dir_all(&dir);
        let file_path = dir.join("test.txt");

        let mut history = FileHistory::new(dir.to_str().unwrap()).unwrap();
        history.record_edit(&file_path, Some("a".to_string()), Some("b".to_string()));
        history.undo().unwrap();
        assert!(history.can_redo());

        // New edit clears redo
        history.record_edit(&file_path, Some("a".to_string()), Some("c".to_string()));
        assert!(!history.can_redo());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_undo_returns_error() {
        let dir = std::env::temp_dir().join("nocode_test_empty_undo");
        let _ = fs::create_dir_all(&dir);
        let mut history = FileHistory::new(dir.to_str().unwrap()).unwrap();
        assert!(history.undo().is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_redo_returns_error() {
        let dir = std::env::temp_dir().join("nocode_test_empty_redo");
        let _ = fs::create_dir_all(&dir);
        let mut history = FileHistory::new(dir.to_str().unwrap()).unwrap();
        assert!(history.redo().is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
