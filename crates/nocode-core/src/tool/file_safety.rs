//! File safety checks — symlink escape prevention, binary detection, size limits.

use std::fs;
use std::path::Path;

/// Maximum file size for read/write operations (10 MB).
pub const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;

/// Check if a file path is safe to operate on.
pub fn validate_file_path(path: &str, workspace: &str) -> Result<(), String> {
    let canonical = match fs::canonicalize(path) {
        Ok(p) => p,
        Err(_) => {
            // File doesn't exist yet — check parent
            if let Some(parent) = Path::new(path).parent() {
                if parent.exists() {
                    match fs::canonicalize(parent) {
                        Ok(p) => p.join(Path::new(path).file_name().unwrap_or_default()),
                        Err(_) => return Ok(()), // Can't resolve, allow
                    }
                } else {
                    return Ok(()); // Parent doesn't exist, will be created
                }
            } else {
                return Ok(());
            }
        }
    };

    // Check symlink escape — resolved path must be within workspace
    let canonical_str = canonical.to_string_lossy();
    if !workspace.is_empty() && !canonical_str.starts_with(workspace) {
        return Err(format!(
            "Path escapes workspace: {path} resolves to {canonical_str}"
        ));
    }

    Ok(())
}

/// Check if a file exceeds the size limit.
pub fn check_file_size(path: &str) -> Result<(), String> {
    if let Ok(metadata) = fs::metadata(path)
        && metadata.len() > MAX_FILE_SIZE
    {
        return Err(format!(
            "File too large: {} bytes (max {} bytes)",
            metadata.len(),
            MAX_FILE_SIZE
        ));
    }
    Ok(())
}

/// Detect if a file is likely binary (contains null bytes in first 8KB).
pub fn is_binary_file(path: &str) -> bool {
    let Ok(data) = fs::read(path) else {
        return false;
    };
    let check_len = data.len().min(8192);
    data[..check_len].contains(&0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn validates_normal_paths() {
        // Paths within workspace should be fine
        assert!(validate_file_path("/tmp/test.txt", "/tmp").is_ok());
    }

    #[test]
    fn detects_binary_files() {
        let tmp = std::env::temp_dir().join("nocode_binary_test.bin");
        let mut f = fs::File::create(&tmp).unwrap();
        f.write_all(&[0x00, 0x01, 0x02, 0x00]).unwrap();
        assert!(is_binary_file(tmp.to_str().unwrap()));
        fs::remove_file(&tmp).unwrap();
    }

    #[test]
    fn detects_text_files() {
        let tmp = std::env::temp_dir().join("nocode_text_test.txt");
        fs::write(&tmp, "hello world\n").unwrap();
        assert!(!is_binary_file(tmp.to_str().unwrap()));
        fs::remove_file(&tmp).unwrap();
    }

    #[test]
    fn check_file_size_passes_small_files() {
        let tmp = std::env::temp_dir().join("nocode_size_test.txt");
        fs::write(&tmp, "small").unwrap();
        assert!(check_file_size(tmp.to_str().unwrap()).is_ok());
        fs::remove_file(&tmp).unwrap();
    }

    #[test]
    fn check_file_size_nonexistent_passes() {
        assert!(check_file_size("/nonexistent/file.txt").is_ok());
    }
}
