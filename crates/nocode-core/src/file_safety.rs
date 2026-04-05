//! File operation safety checks: symlink escape, binary detection, size limits.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Maximum file size allowed for read/write operations (10 MB).
pub const MAX_FILE_SIZE: usize = 10_485_760;

/// Number of bytes to inspect for binary detection.
pub const BINARY_CHECK_SIZE: usize = 8192;

/// Resolve all symlinks in `path` and verify the canonical result lives under `cwd`.
/// If the file does not exist on disk (e.g. mock/virtual host), returns the original path.
pub fn check_symlink_escape(path: &Path, cwd: &Path) -> Result<PathBuf, String> {
    let canonical = match fs::canonicalize(path) {
        Ok(c) => c,
        Err(_) => {
            // File doesn't exist on disk — skip symlink check (resolve_path already
            // validated the logical boundary). Return the path as-is.
            return Ok(path.to_path_buf());
        }
    };
    let canonical_cwd = match fs::canonicalize(cwd) {
        Ok(c) => c,
        Err(_) => return Ok(path.to_path_buf()),
    };
    if !canonical.starts_with(&canonical_cwd) {
        return Err(format!(
            "path escapes cwd via symlink: {} resolves to {}",
            path.display(),
            canonical.display()
        ));
    }
    Ok(canonical)
}

/// Return `true` if the file appears to be binary (contains NUL bytes in the first chunk).
pub fn is_binary_file(path: &Path) -> Result<bool, String> {
    let mut file = fs::File::open(path)
        .map_err(|e| format!("failed to open {} for binary check: {e}", path.display()))?;
    let mut buf = vec![0u8; BINARY_CHECK_SIZE];
    let n = file
        .read(&mut buf)
        .map_err(|e| format!("failed to read {} for binary check: {e}", path.display()))?;
    Ok(buf[..n].contains(&0x00))
}

/// Check that the file does not exceed `MAX_FILE_SIZE`. Returns the size on success.
/// If the file does not exist on disk, returns Ok(0) — the actual read will fail later.
pub fn check_file_size(path: &Path) -> Result<u64, String> {
    let meta = match fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return Ok(0), // File not on disk (mock host); skip size check.
    };
    let size = meta.len();
    if size > MAX_FILE_SIZE as u64 {
        return Err(format!(
            "file too large: {} is {} bytes (limit {})",
            path.display(),
            size,
            MAX_FILE_SIZE
        ));
    }
    Ok(size)
}
/// Validate a file for reading: symlink check, size check, binary warning.
/// Returns the canonical path. If the file is binary, returns `Ok` but logs a warning
/// in the path (caller can inspect via `is_binary_file` separately).
pub fn validate_read_target(path: &Path, cwd: &Path) -> Result<PathBuf, String> {
    let canonical = check_symlink_escape(path, cwd)?;
    check_file_size(&canonical)?;
    // Binary check: warn but don't block reads.
    match is_binary_file(&canonical) {
        Ok(true) => {
            // We still return Ok — caller may choose to warn the user.
        }
        Ok(false) => {}
        Err(_) => {
            // If we can't check, proceed anyway — the read itself will fail if needed.
        }
    }
    Ok(canonical)
}

/// Validate a file for writing: symlink check on existing files, parent-in-cwd check.
pub fn validate_write_target(path: &Path, cwd: &Path) -> Result<PathBuf, String> {
    let canonical_cwd = match fs::canonicalize(cwd) {
        Ok(c) => c,
        Err(_) => {
            // cwd not on disk (mock host); skip write validation.
            return Ok(path.to_path_buf());
        }
    };

    if path.exists() {
        // Existing file: full symlink escape check.
        let canonical = check_symlink_escape(path, cwd)?;
        return Ok(canonical);
    }

    // New file: verify parent directory is within cwd.
    let parent = path
        .parent()
        .ok_or_else(|| format!("path has no parent: {}", path.display()))?;
    if !parent.exists() {
        return Err(format!(
            "parent directory does not exist: {}",
            parent.display()
        ));
    }
    let canonical_parent = fs::canonicalize(parent)
        .map_err(|e| format!("failed to canonicalize parent {}: {e}", parent.display()))?;
    if !canonical_parent.starts_with(&canonical_cwd) {
        return Err(format!(
            "write target parent escapes cwd: {} resolves to {}",
            parent.display(),
            canonical_parent.display()
        ));
    }
    let file_name = path
        .file_name()
        .ok_or_else(|| format!("path has no file name: {}", path.display()))?;
    Ok(canonical_parent.join(file_name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn setup_test_dir() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let cwd = dir.path().to_path_buf();
        (dir, cwd)
    }

    #[test]
    fn normal_file_passes_validation() {
        let (_dir, cwd) = setup_test_dir();
        let file = cwd.join("hello.txt");
        fs::write(&file, "hello world").unwrap();

        let result = validate_read_target(&file, &cwd);
        assert!(result.is_ok(), "normal file should pass: {result:?}");
    }

    #[test]
    fn file_over_size_limit_rejected() {
        let (_dir, cwd) = setup_test_dir();
        let file = cwd.join("big.bin");
        // Create a sparse file that reports > MAX_FILE_SIZE.
        let f = fs::File::create(&file).unwrap();
        f.set_len(MAX_FILE_SIZE as u64 + 1).unwrap();

        let result = validate_read_target(&file, &cwd);
        assert!(result.is_err(), "oversized file should be rejected");
        assert!(result.unwrap_err().contains("file too large"));
    }

    #[test]
    fn binary_file_detected() {
        let (_dir, cwd) = setup_test_dir();
        let file = cwd.join("data.bin");
        let mut f = fs::File::create(&file).unwrap();
        f.write_all(&[0x89, 0x50, 0x4E, 0x47, 0x00, 0xFF, 0xAB])
            .unwrap();

        let result = is_binary_file(&file);
        assert!(result.is_ok());
        assert!(result.unwrap(), "file with NUL byte should be binary");
    }

    #[test]
    fn path_outside_cwd_rejected() {
        let (_dir, cwd) = setup_test_dir();
        // Create a file outside cwd.
        let outside = tempfile::tempdir().expect("failed to create outer dir");
        let outside_file = outside.path().join("secret.txt");
        fs::write(&outside_file, "secret").unwrap();

        let result = validate_read_target(&outside_file, &cwd);
        assert!(result.is_err(), "path outside cwd should be rejected");
        assert!(result.unwrap_err().contains("escapes cwd"));
    }
}
