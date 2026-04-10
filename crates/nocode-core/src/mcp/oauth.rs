//! MCP OAuth — OAuth authentication for MCP servers.
//!
//! Wraps the generic OAuth client for MCP-specific flows.
//! MCP servers can require OAuth tokens for tool execution.

use crate::auth::oauth::{OAuthClient, OAuthConfig, TokenResponse};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// MCP-specific OAuth configuration (stored per server).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpOAuthConfig {
    pub server_name: String,
    pub oauth: OAuthConfig,
}

/// Cached token for an MCP server.
#[derive(Debug, Clone)]
pub struct CachedToken {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<i64>,
}

impl CachedToken {
    pub fn from_response(resp: &TokenResponse) -> Self {
        let expires_at = resp.expires_in.map(|secs| {
            chrono::Utc::now().timestamp() + secs as i64
        });
        Self {
            access_token: resp.access_token.clone(),
            refresh_token: resp.refresh_token.clone(),
            expires_at,
        }
    }

    pub fn is_expired(&self) -> bool {
        self.expires_at
            .is_some_and(|exp| chrono::Utc::now().timestamp() > exp)
    }
}

/// MCP OAuth manager — handles token lifecycle for MCP servers.
pub struct McpOAuthManager {
    configs: HashMap<String, McpOAuthConfig>,
    tokens: HashMap<String, CachedToken>,
}

impl McpOAuthManager {
    pub fn new() -> Self {
        Self {
            configs: HashMap::new(),
            tokens: HashMap::new(),
        }
    }
// APPEND_REST

    /// Register OAuth config for an MCP server.
    pub fn register(&mut self, config: McpOAuthConfig) {
        self.configs.insert(config.server_name.clone(), config);
    }

    /// Start a full OAuth flow: spin up a localhost callback server, open the
    /// browser, wait for the redirect, exchange the code, and cache the token.
    ///
    /// Returns the cached access token on success.
    pub fn start_oauth_flow(
        &mut self,
        server_name: &str,
        timeout_secs: u64,
    ) -> Result<String, String> {
        // 1. Bind a random port for the callback
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .map_err(|e| format!("Failed to bind callback port: {e}"))?;
        let port = listener.local_addr()
            .map_err(|e| format!("Failed to get local addr: {e}"))?
            .port();

        // 2. Dynamically update redirect_uri to match the bound port
        let redirect_uri = format!("http://127.0.0.1:{port}/callback");

        // 3. Temporarily patch the config's redirect_uri for URL generation
        let config = self.configs.get(server_name)
            .ok_or_else(|| format!("No OAuth config for server '{server_name}'"))?;
        let mut patched_config = config.oauth.clone();
        patched_config.redirect_uri = redirect_uri.clone();

        let client = OAuthClient::new(patched_config);
        let state = format!("mcp-oauth-{server_name}");
        let (url, pkce) = client.authorize_url(&state);
        let pkce_verifier = pkce.verifier;

        // 4. Open browser
        let browser_opened = open_browser(&url);

        // 5. Wait for callback with timeout
        listener.set_nonblocking(false)
            .map_err(|e| format!("Failed to set blocking mode: {e}"))?;

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
        let mut code_opt: Option<String> = None;

        // Set read timeout on the listener so we can check deadline
        listener.set_ttl(30).ok();
        let accept_timeout = std::time::Duration::from_secs(5);
        listener.set_nonblocking(false).ok();

        while std::time::Instant::now() < deadline {
            // Set a per-accept timeout
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            let _wait_time = remaining.min(accept_timeout);

            match listener.accept() {
                Ok((mut stream, _addr)) => {
                    // Read the HTTP request
                    let mut buf = [0u8; 4096];
                    let n = std::io::Read::read(&mut stream, &mut buf)
                        .unwrap_or(0);
                    let request_str = String::from_utf8_lossy(&buf[..n]);

                    // Send a "you can close this page" response
                    let response = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n<html><body><h1>Authorization successful!</h1><p>You can close this tab now.</p></body></html>";
                    let _ = std::io::Write::write_all(&mut stream, response.as_bytes());
                    let _ = stream.shutdown(std::net::Shutdown::Both);

                    // Extract code from query string
                    if let Some(code) = extract_code_from_request(&request_str) {
                        code_opt = Some(code);
                        break;
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                Err(_) => {
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
            }

            if !browser_opened {
                // If we couldn't open the browser, give the user the URL to visit manually
                // but still wait for the callback
                eprintln!("Please open this URL in your browser:\n{url}");
                // Only print once — set a flag to avoid repeated messages
                break;
            }
        }

        let code = code_opt
            .ok_or_else(|| "OAuth flow timed out — no callback received".to_string())?;

        // 6. Exchange code for token
        // Need to use patched config for exchange too
        let exchange_config = self.configs.get(server_name)
            .ok_or_else(|| format!("No OAuth config for server '{server_name}'"))?;
        let mut exchange_config = exchange_config.oauth.clone();
        exchange_config.redirect_uri = redirect_uri;
        let exchange_client = OAuthClient::new(exchange_config);
        let resp = exchange_client.exchange_code(&code, &pkce_verifier)?;

        // 7. Cache token
        let token = resp.access_token.clone();
        self.tokens.insert(server_name.to_string(), CachedToken::from_response(&resp));

        Ok(token)
    }

    /// Get the authorization URL for an MCP server.
    pub fn authorize_url(&self, server_name: &str, state: &str) -> Result<(String, String), String> {
        let config = self.configs.get(server_name)
            .ok_or_else(|| format!("No OAuth config for server '{server_name}'"))?;
        let client = OAuthClient::new(config.oauth.clone());
        let (url, pkce) = client.authorize_url(state);
        Ok((url, pkce.verifier))
    }

    /// Exchange an authorization code for tokens and cache them.
    pub fn exchange_code(
        &mut self,
        server_name: &str,
        code: &str,
        pkce_verifier: &str,
    ) -> Result<(), String> {
        let config = self.configs.get(server_name)
            .ok_or_else(|| format!("No OAuth config for server '{server_name}'"))?;
        let client = OAuthClient::new(config.oauth.clone());
        let resp = client.exchange_code(code, pkce_verifier)?;
        self.tokens.insert(server_name.to_string(), CachedToken::from_response(&resp));
        Ok(())
    }

    /// Get a valid access token for an MCP server (refreshing if needed).
    pub fn get_token(&mut self, server_name: &str) -> Result<String, String> {
        let cached = self.tokens.get(server_name)
            .ok_or_else(|| format!("No token for server '{server_name}' — run OAuth flow first"))?
            .clone();

        if !cached.is_expired() {
            return Ok(cached.access_token);
        }

        // Try refresh
        let Some(refresh_token) = &cached.refresh_token else {
            return Err(format!("Token expired and no refresh token for '{server_name}'"));
        };

        let config = self.configs.get(server_name)
            .ok_or_else(|| format!("No OAuth config for server '{server_name}'"))?;
        let client = OAuthClient::new(config.oauth.clone());
        let resp = client.refresh_token(refresh_token)?;
        let new_cached = CachedToken::from_response(&resp);
        let token = new_cached.access_token.clone();
        self.tokens.insert(server_name.to_string(), new_cached);
        Ok(token)
    }

    /// Check if a server has a valid (non-expired) token.
    pub fn has_valid_token(&self, server_name: &str) -> bool {
        self.tokens.get(server_name)
            .is_some_and(|t| !t.is_expired())
    }

    /// Revoke/clear cached token for a server.
    pub fn revoke(&mut self, server_name: &str) -> bool {
        self.tokens.remove(server_name).is_some()
    }

    /// List servers with OAuth configured.
    pub fn configured_servers(&self) -> Vec<&str> {
        self.configs.keys().map(String::as_str).collect()
    }

    /// List servers with active tokens.
    pub fn authenticated_servers(&self) -> Vec<&str> {
        self.tokens
            .iter()
            .filter(|(_, t)| !t.is_expired())
            .map(|(k, _)| k.as_str())
            .collect()
    }
}

impl Default for McpOAuthManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Try to open a URL in the system's default browser.
/// Returns true if a browser command was found and executed.
fn open_browser(url: &str) -> bool {
    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(target_os = "windows")]
    let cmd = "explorer";
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let cmd = "xdg-open";

    // Try the platform command first, then fallback to common browsers
    let candidates = [
        cmd,
        "python3", // python3 -m webbrowser
    ];

    for candidate in &candidates {
        if which_command(candidate) {
            let result = match *candidate {
                "python3" => std::process::Command::new("python3")
                    .args(["-m", "webbrowser", url])
                    .spawn(),
                _ => std::process::Command::new(candidate)
                    .arg(url)
                    .spawn(),
            };
            if result.is_ok() {
                return true;
            }
        }
    }

    false
}

/// Check if a command exists on the system PATH.
fn which_command(cmd: &str) -> bool {
    std::process::Command::new("which")
        .arg(cmd)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Extract the authorization code from an HTTP request line.
/// Expects GET /callback?code=xxx&state=yyy
fn extract_code_from_request(request: &str) -> Option<String> {
    // Parse the request line: "GET /callback?code=xxx&state=yyy HTTP/1.1"
    let first_line = request.lines().next()?;
    let path = first_line.split(' ').nth(1)?;
    let query = path.split('?').nth(1)?;

    for pair in query.split('&') {
        let (key, value) = pair.split_once('=')?;
        if key == "code" {
            return Some(urlencoding::decode(value).ok()?.into_owned());
        }
    }

    None
}

/// Global singleton MCP OAuth manager.
static GLOBAL_MCP_OAUTH: OnceLock<Arc<Mutex<McpOAuthManager>>> = OnceLock::new();

pub fn global_mcp_oauth() -> &'static Arc<Mutex<McpOAuthManager>> {
    GLOBAL_MCP_OAUTH.get_or_init(|| Arc::new(Mutex::new(McpOAuthManager::new())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_mcp_oauth_config() -> McpOAuthConfig {
        McpOAuthConfig {
            server_name: "github-mcp".to_string(),
            oauth: OAuthConfig {
                client_id: "mcp-client".to_string(),
                client_secret: None,
                authorize_url: "https://github.com/login/oauth/authorize".to_string(),
                token_url: "https://github.com/login/oauth/access_token".to_string(),
                redirect_uri: "http://localhost:8080/callback".to_string(),
                scopes: vec!["repo".to_string()],
            },
        }
    }

    #[test]
    fn register_and_authorize() {
        let mut mgr = McpOAuthManager::new();
        mgr.register(test_mcp_oauth_config());
        let (url, verifier) = mgr.authorize_url("github-mcp", "state1").unwrap();
        assert!(url.contains("github.com"));
        assert!(url.contains("response_type=code"));
        assert!(!verifier.is_empty());
    }

    #[test]
    fn authorize_unknown_server_fails() {
        let mgr = McpOAuthManager::new();
        assert!(mgr.authorize_url("ghost", "s").is_err());
    }

    #[test]
    fn cached_token_expiry() {
        let fresh = CachedToken {
            access_token: "at".to_string(),
            refresh_token: None,
            expires_at: Some(chrono::Utc::now().timestamp() + 3600),
        };
        assert!(!fresh.is_expired());

        let expired = CachedToken {
            access_token: "at".to_string(),
            refresh_token: None,
            expires_at: Some(chrono::Utc::now().timestamp() - 10),
        };
        assert!(expired.is_expired());

        let no_expiry = CachedToken {
            access_token: "at".to_string(),
            refresh_token: None,
            expires_at: None,
        };
        assert!(!no_expiry.is_expired());
    }

    #[test]
    fn has_valid_token_false_when_empty() {
        let mgr = McpOAuthManager::new();
        assert!(!mgr.has_valid_token("any"));
    }

    #[test]
    fn revoke_clears_token() {
        let mut mgr = McpOAuthManager::new();
        mgr.tokens.insert("srv".to_string(), CachedToken {
            access_token: "at".to_string(),
            refresh_token: None,
            expires_at: None,
        });
        assert!(mgr.has_valid_token("srv"));
        assert!(mgr.revoke("srv"));
        assert!(!mgr.has_valid_token("srv"));
    }

    #[test]
    fn configured_and_authenticated_servers() {
        let mut mgr = McpOAuthManager::new();
        mgr.register(test_mcp_oauth_config());
        assert_eq!(mgr.configured_servers().len(), 1);
        assert!(mgr.authenticated_servers().is_empty());

        mgr.tokens.insert("github-mcp".to_string(), CachedToken {
            access_token: "at".to_string(),
            refresh_token: None,
            expires_at: Some(chrono::Utc::now().timestamp() + 3600),
        });
        assert_eq!(mgr.authenticated_servers().len(), 1);
    }

    #[test]
    fn get_token_no_token_fails() {
        let mut mgr = McpOAuthManager::new();
        mgr.register(test_mcp_oauth_config());
        assert!(mgr.get_token("github-mcp").is_err());
    }

    #[test]
    fn get_token_returns_cached() {
        let mut mgr = McpOAuthManager::new();
        mgr.tokens.insert("srv".to_string(), CachedToken {
            access_token: "my-token".to_string(),
            refresh_token: None,
            expires_at: Some(chrono::Utc::now().timestamp() + 3600),
        });
        assert_eq!(mgr.get_token("srv").unwrap(), "my-token");
    }

    #[test]
    fn mcp_oauth_config_serde() {
        let config = test_mcp_oauth_config();
        let json = serde_json::to_string(&config).unwrap();
        let parsed: McpOAuthConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.server_name, "github-mcp");
    }
}
