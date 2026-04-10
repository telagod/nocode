//! Update checker — checks GitHub releases for newer versions.
//!
//! Disabled by env `NOCODE_DISABLE_UPDATE_CHECK=1` or feature flag `auto_update=false`.
//! Caches last check timestamp to avoid spamming the API (max once per 24h).

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Version comparison result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    /// Current version is up to date.
    UpToDate,
    /// A newer version is available.
    UpdateAvailable {
        current: String,
        latest: String,
        download_url: String,
    },
    /// Check was skipped (disabled, rate-limited, or error).
    Skipped(String),
}

/// Cached check state persisted to disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateCheckCache {
    pub last_check_epoch: i64,
    pub latest_version: String,
    pub download_url: String,
}

/// GitHub release response (minimal fields).
#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: String,
}

/// Update checker configuration.
pub struct UpdateChecker {
    current_version: String,
    cache_path: PathBuf,
    /// Minimum seconds between checks (default 86400 = 24h).
    check_interval_secs: i64,
    repo: String,
}

// APPEND_IMPL

impl UpdateChecker {
    pub fn new(current_version: &str, cache_path: &str, repo: &str) -> Self {
        Self {
            current_version: current_version.to_string(),
            cache_path: PathBuf::from(cache_path),
            check_interval_secs: 86400,
            repo: repo.to_string(),
        }
    }

    pub fn with_interval(mut self, secs: i64) -> Self {
        self.check_interval_secs = secs;
        self
    }

    /// Check if updates are disabled via env var.
    pub fn is_disabled() -> bool {
        std::env::var("NOCODE_DISABLE_UPDATE_CHECK")
            .is_ok_and(|v| matches!(v.as_str(), "1" | "true" | "yes"))
    }

    /// Check for updates. Returns cached result if within interval.
    pub fn check(&self) -> UpdateStatus {
        if Self::is_disabled() {
            return UpdateStatus::Skipped("disabled by env".to_string());
        }

        // Check cache first
        if let Some(cached) = self.load_cache() {
            let now = chrono::Utc::now().timestamp();
            if now - cached.last_check_epoch < self.check_interval_secs {
                return self.compare_version(&cached.latest_version, &cached.download_url);
            }
        }

        // Fetch from GitHub (blocking — intended for startup check)
        match self.fetch_latest() {
            Ok((version, url)) => {
                self.save_cache(&version, &url);
                self.compare_version(&version, &url)
            }
            Err(e) => UpdateStatus::Skipped(format!("fetch error: {e}")),
        }
    }

    /// Check using only the cache (no network). For offline environments.
    pub fn check_cached_only(&self) -> UpdateStatus {
        match self.load_cache() {
            Some(cached) => self.compare_version(&cached.latest_version, &cached.download_url),
            None => UpdateStatus::Skipped("no cache".to_string()),
        }
    }

    /// Compare semver strings. Returns UpdateAvailable if latest > current.
    fn compare_version(&self, latest: &str, url: &str) -> UpdateStatus {
        let latest_clean = latest.trim_start_matches('v');
        let current_clean = self.current_version.trim_start_matches('v');

        if semver_gt(latest_clean, current_clean) {
            UpdateStatus::UpdateAvailable {
                current: self.current_version.clone(),
                latest: latest.to_string(),
                download_url: url.to_string(),
            }
        } else {
            UpdateStatus::UpToDate
        }
    }

    fn fetch_latest(&self) -> Result<(String, String), String> {
        let url = format!("https://api.github.com/repos/{}/releases/latest", self.repo);
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .user_agent("nocode-update-checker")
            .build()
            .map_err(|e| format!("client error: {e}"))?;

        let resp = client
            .get(&url)
            .send()
            .map_err(|e| format!("request error: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()));
        }
        let release: GitHubRelease = resp.json().map_err(|e| format!("parse error: {e}"))?;
        Ok((release.tag_name, release.html_url))
    }

    fn load_cache(&self) -> Option<UpdateCheckCache> {
        let raw = fs::read_to_string(&self.cache_path).ok()?;
        serde_json::from_str(&raw).ok()
    }

    fn save_cache(&self, version: &str, url: &str) {
        let cache = UpdateCheckCache {
            last_check_epoch: chrono::Utc::now().timestamp(),
            latest_version: version.to_string(),
            download_url: url.to_string(),
        };
        if let Some(parent) = self.cache_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(
            &self.cache_path,
            serde_json::to_string_pretty(&cache).unwrap_or_default(),
        );
    }
}

/// Simple semver greater-than comparison (major.minor.patch).
fn semver_gt(a: &str, b: &str) -> bool {
    let parse = |s: &str| -> (u32, u32, u32) {
        let parts: Vec<u32> = s.split('.').filter_map(|p| p.parse().ok()).collect();
        (
            parts.first().copied().unwrap_or(0),
            parts.get(1).copied().unwrap_or(0),
            parts.get(2).copied().unwrap_or(0),
        )
    };
    parse(a) > parse(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_gt_basic() {
        assert!(semver_gt("1.0.1", "1.0.0"));
        assert!(semver_gt("1.1.0", "1.0.9"));
        assert!(semver_gt("2.0.0", "1.9.9"));
        assert!(!semver_gt("1.0.0", "1.0.0"));
        assert!(!semver_gt("0.9.0", "1.0.0"));
    }

    #[test]
    fn semver_gt_with_v_prefix() {
        // semver_gt itself doesn't strip v, but compare_version does
        let checker = UpdateChecker::new("0.2.12", "/tmp/nocode_uc_test.json", "test/repo");
        let status = checker.compare_version("v0.2.13", "https://example.com");
        assert!(matches!(status, UpdateStatus::UpdateAvailable { .. }));
    }

    #[test]
    fn up_to_date() {
        let checker = UpdateChecker::new("0.2.12", "/tmp/nocode_uc_test2.json", "test/repo");
        let status = checker.compare_version("v0.2.12", "https://example.com");
        assert_eq!(status, UpdateStatus::UpToDate);
    }

    #[test]
    fn older_version_is_up_to_date() {
        let checker = UpdateChecker::new("0.3.0", "/tmp/nocode_uc_test3.json", "test/repo");
        let status = checker.compare_version("v0.2.12", "https://example.com");
        assert_eq!(status, UpdateStatus::UpToDate);
    }

    #[test]
    fn cache_save_and_load() {
        let tmp = format!("/tmp/nocode_uc_cache_{}.json", std::process::id());
        let _ = fs::remove_file(&tmp);
        let checker = UpdateChecker::new("0.2.12", &tmp, "test/repo");
        checker.save_cache("v0.3.0", "https://example.com/release");
        let cached = checker.load_cache().unwrap();
        assert_eq!(cached.latest_version, "v0.3.0");
        assert_eq!(cached.download_url, "https://example.com/release");
        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn check_cached_only_no_cache() {
        let tmp = format!("/tmp/nocode_uc_nocache_{}.json", std::process::id());
        let _ = fs::remove_file(&tmp);
        let checker = UpdateChecker::new("0.2.12", &tmp, "test/repo");
        let status = checker.check_cached_only();
        assert!(matches!(status, UpdateStatus::Skipped(_)));
    }

    #[test]
    fn check_cached_only_with_cache() {
        let tmp = format!("/tmp/nocode_uc_cached_{}.json", std::process::id());
        let _ = fs::remove_file(&tmp);
        let checker = UpdateChecker::new("0.2.12", &tmp, "test/repo");
        checker.save_cache("v0.3.0", "https://example.com");
        let status = checker.check_cached_only();
        assert!(matches!(status, UpdateStatus::UpdateAvailable { .. }));
        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn update_status_debug() {
        let s = UpdateStatus::UpToDate;
        assert_eq!(format!("{s:?}"), "UpToDate");
    }
}
