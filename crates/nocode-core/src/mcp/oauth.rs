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
