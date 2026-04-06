//! Linux sandbox detection and capability-aware degradation.

use std::env;
use std::fs;
use std::process::Command;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilesystemIsolationMode {
    Off,
    WorkspaceOnly,
    AllowList,
}

#[derive(Debug, Clone)]
pub struct SandboxRequest {
    pub enabled: bool,
    pub filesystem_mode: FilesystemIsolationMode,
    pub network_isolation: bool,
    pub allowed_mounts: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ContainerEnvironment {
    pub in_container: bool,
    pub markers: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SandboxStatus {
    pub requested: bool,
    pub supported: bool,
    pub active: bool,
    pub filesystem_mode: FilesystemIsolationMode,
    pub network_isolated: bool,
    pub container: ContainerEnvironment,
    pub fallback_reasons: Vec<String>,
}

// ---------------------------------------------------------------------------
// Detection helpers
// ---------------------------------------------------------------------------

/// Detect whether we are running inside a container by probing well-known
/// marker files and environment variables.
pub fn detect_container() -> ContainerEnvironment {
    let mut markers = Vec::new();

    if fs::metadata("/.dockerenv").is_ok() {
        markers.push("/.dockerenv".to_string());
    }
    if fs::metadata("/run/.containerenv").is_ok() {
        markers.push("/run/.containerenv".to_string());
    }
    if env::var("CONTAINER").is_ok() {
        markers.push("CONTAINER env var".to_string());
    }

    let in_container = !markers.is_empty();
    ContainerEnvironment {
        in_container,
        markers,
    }
}

/// Check whether the `unshare` command is available on this system.
pub fn check_namespace_support() -> bool {
    Command::new("unshare")
        .arg("--help")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
}

/// Build a [`SandboxStatus`] by combining the user request with runtime
/// capability detection.  When the requested isolation level is not
/// achievable the function degrades gracefully and records the reason.
pub fn resolve_sandbox_status(request: &SandboxRequest) -> SandboxStatus {
    let container = detect_container();
    let ns_support = check_namespace_support();

    let mut fallback_reasons: Vec<String> = Vec::new();
    let mut supported = true;
    let mut active = request.enabled;
    let mut fs_mode = request.filesystem_mode;
    let mut net_isolated = request.network_isolation;

    if !request.enabled {
        active = false;
        supported = true;
        return SandboxStatus {
            requested: false,
            supported,
            active,
            filesystem_mode: FilesystemIsolationMode::Off,
            network_isolated: false,
            container,
            fallback_reasons,
        };
    }

    // Namespace support is required for filesystem and network isolation.
    if !ns_support {
        supported = false;
        active = false;
        fs_mode = FilesystemIsolationMode::Off;
        net_isolated = false;
        fallback_reasons.push("unshare not available — namespace isolation disabled".into());
    }

    // Inside a container we may lack CAP_SYS_ADMIN for nested namespaces.
    if container.in_container && ns_support {
        // We can still *try*, but flag the risk.
        fallback_reasons
            .push("running inside a container — nested namespace support may be limited".into());
    }

    SandboxStatus {
        requested: request.enabled,
        supported,
        active,
        filesystem_mode: fs_mode,
        network_isolated: net_isolated,
        container,
        fallback_reasons,
    }
}

/// Render a human-readable sandbox status report.
pub fn render_sandbox_report(status: &SandboxStatus) -> String {
    let mut lines: Vec<String> = Vec::new();

    lines.push(format!("Sandbox requested : {}", status.requested));
    lines.push(format!("Sandbox supported : {}", status.supported));
    lines.push(format!("Sandbox active    : {}", status.active));
    lines.push(format!("Filesystem mode   : {:?}", status.filesystem_mode));
    lines.push(format!("Network isolated  : {}", status.network_isolated));

    if status.container.in_container {
        lines.push(format!(
            "Container detected: yes ({})",
            status.container.markers.join(", ")
        ));
    } else {
        lines.push("Container detected: no".to_string());
    }

    if !status.fallback_reasons.is_empty() {
        lines.push("Fallback reasons:".to_string());
        for reason in &status.fallback_reasons {
            lines.push(format!("  - {reason}"));
        }
    }

    lines.join("\n")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_container_returns_valid_struct() {
        let env = detect_container();
        // We cannot assert the exact value because CI may or may not be
        // containerised, but the struct must be well-formed.
        if env.in_container {
            assert!(!env.markers.is_empty());
        } else {
            assert!(env.markers.is_empty());
        }
    }

    #[test]
    fn check_namespace_support_returns_bool() {
        // Just ensure it does not panic.
        let _ = check_namespace_support();
    }

    #[test]
    fn resolve_disabled_request() {
        let req = SandboxRequest {
            enabled: false,
            filesystem_mode: FilesystemIsolationMode::WorkspaceOnly,
            network_isolation: true,
            allowed_mounts: vec![],
        };
        let st = resolve_sandbox_status(&req);
        assert!(!st.requested);
        assert!(!st.active);
        assert_eq!(st.filesystem_mode, FilesystemIsolationMode::Off);
        assert!(!st.network_isolated);
    }

    #[test]
    fn resolve_enabled_request_populates_fields() {
        let req = SandboxRequest {
            enabled: true,
            filesystem_mode: FilesystemIsolationMode::AllowList,
            network_isolation: true,
            allowed_mounts: vec!["/tmp".into()],
        };
        let st = resolve_sandbox_status(&req);
        assert!(st.requested);
        // active/supported depend on the host, but requested must be true.
    }

    #[test]
    fn render_report_contains_key_lines() {
        let status = SandboxStatus {
            requested: true,
            supported: true,
            active: true,
            filesystem_mode: FilesystemIsolationMode::WorkspaceOnly,
            network_isolated: true,
            container: ContainerEnvironment {
                in_container: false,
                markers: vec![],
            },
            fallback_reasons: vec!["test reason".into()],
        };
        let report = render_sandbox_report(&status);
        assert!(report.contains("Sandbox requested : true"));
        assert!(report.contains("Sandbox active    : true"));
        assert!(report.contains("WorkspaceOnly"));
        assert!(report.contains("test reason"));
    }

    #[test]
    fn render_report_shows_container_markers() {
        let status = SandboxStatus {
            requested: false,
            supported: false,
            active: false,
            filesystem_mode: FilesystemIsolationMode::Off,
            network_isolated: false,
            container: ContainerEnvironment {
                in_container: true,
                markers: vec!["/.dockerenv".into()],
            },
            fallback_reasons: vec![],
        };
        let report = render_sandbox_report(&status);
        assert!(report.contains("Container detected: yes"));
        assert!(report.contains("/.dockerenv"));
    }
}
