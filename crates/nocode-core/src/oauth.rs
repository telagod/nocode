//! OAuth 2.0 PKCE helpers — code verifier/challenge generation, token
//! persistence, and authorization URL construction.

use serde::{Deserialize, Serialize};
use std::fmt::Write as FmtWrite;
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct PkceCodePair {
    pub verifier: String,
    pub challenge: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthTokenSet {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<u64>,
    pub token_type: String,
}

#[derive(Debug, Clone)]
pub struct OAuthConfig {
    pub client_id: String,
    pub redirect_uri: String,
    pub auth_url: String,
    pub token_url: String,
    pub scopes: Vec<String>,
}

// ---------------------------------------------------------------------------
// Pseudo-random (no `rand` crate)
// ---------------------------------------------------------------------------

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn pseudo_random_bytes(len: usize) -> Vec<u8> {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let nanos = ts.as_nanos() as u64;
    let tid = std::thread::current().id();
    let tid_hash = format!("{tid:?}");
    let cnt = COUNTER.fetch_add(1, Ordering::Relaxed);

    // Simple xorshift-style mixing seeded from timestamp + thread id + counter.
    let mut seed: u64 = nanos
        ^ cnt.wrapping_mul(6_364_136_223_846_793_005)
        ^ tid_hash.len() as u64;
    for b in tid_hash.bytes() {
        seed ^= (b as u64).wrapping_mul(2_654_435_761);
    }

    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed = seed.wrapping_add(i as u64);
        out.push(seed as u8);
    }
    out
}

// ---------------------------------------------------------------------------
// Base64-url (no padding) — minimal inline implementation
// ---------------------------------------------------------------------------

const B64URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn base64url_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64URL[((triple >> 18) & 0x3F) as usize] as char);
        out.push(B64URL[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(B64URL[((triple >> 6) & 0x3F) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(B64URL[(triple & 0x3F) as usize] as char);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Minimal SHA-256 (standalone, no external crate)
// ---------------------------------------------------------------------------

fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5,
        0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
        0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc,
        0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3,
        0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5,
        0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
        0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];

    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
        0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];

    // Pre-processing: padding
    let bit_len = (data.len() as u64) * 8;
    let mut msg = data.to_vec();
    msg.push(0x80);
    while (msg.len() % 64) != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    // Process each 512-bit block
    for block in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);

        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut digest = [0u8; 32];
    for (i, val) in h.iter().enumerate() {
        digest[i * 4..i * 4 + 4].copy_from_slice(&val.to_be_bytes());
    }
    digest
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Generate a PKCE code verifier (43-128 unreserved characters) and its
/// SHA-256 base64url-encoded challenge.
pub fn generate_pkce_pair() -> PkceCodePair {
    // RFC 7636 requires 43-128 characters from the unreserved set.
    // We pick 64 random bytes and base64url-encode them (yields 86 chars).
    let raw = pseudo_random_bytes(64);
    let verifier = base64url_encode(&raw);
    // Truncate to 128 if somehow longer (won't happen with 64 bytes, but be safe).
    let verifier = if verifier.len() > 128 {
        verifier[..128].to_string()
    } else {
        verifier
    };

    let digest = sha256(verifier.as_bytes());
    let challenge = base64url_encode(&digest);

    PkceCodePair {
        verifier,
        challenge,
    }
}

/// Generate a random 32-character hex string for the OAuth `state` parameter.
pub fn generate_state() -> String {
    let raw = pseudo_random_bytes(16);
    let mut hex = String::with_capacity(32);
    for b in &raw {
        let _ = write!(hex, "{b:02x}");
    }
    hex
}

/// Build the full authorization URL including PKCE and state parameters.
pub fn build_authorization_url(
    config: &OAuthConfig,
    pkce: &PkceCodePair,
    state: &str,
) -> String {
    let scopes = config.scopes.join(" ");
    format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&state={}&code_challenge={}&code_challenge_method=S256&scope={}",
        config.auth_url,
        url_encode(&config.client_id),
        url_encode(&config.redirect_uri),
        url_encode(state),
        url_encode(&pkce.challenge),
        url_encode(&scopes),
    )
}

/// Returns `true` if the token has an `expires_at` timestamp that is in the
/// past (compared to the current system time).
pub fn token_is_expired(token: &OAuthTokenSet) -> bool {
    match token.expires_at {
        Some(exp) => {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            now >= exp
        }
        None => false,
    }
}

/// Persist an [`OAuthTokenSet`] to disk as JSON.
pub fn persist_tokens(path: &str, tokens: &OAuthTokenSet) -> Result<(), String> {
    let json = serde_json::to_string_pretty(tokens).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())
}

/// Load an [`OAuthTokenSet`] from disk.  Returns `Ok(None)` when the file
/// does not exist.
pub fn load_tokens(path: &str) -> Result<Option<OAuthTokenSet>, String> {
    match fs::read_to_string(path) {
        Ok(contents) => {
            let tokens: OAuthTokenSet =
                serde_json::from_str(&contents).map_err(|e| e.to_string())?;
            Ok(Some(tokens))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Minimal percent-encoding for URL query values
// ---------------------------------------------------------------------------

fn url_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len() * 2);
    for b in input.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_verifier_length_in_range() {
        let pair = generate_pkce_pair();
        assert!(
            pair.verifier.len() >= 43,
            "verifier too short: {}",
            pair.verifier.len()
        );
        assert!(
            pair.verifier.len() <= 128,
            "verifier too long: {}",
            pair.verifier.len()
        );
    }

    #[test]
    fn pkce_challenge_is_base64url() {
        let pair = generate_pkce_pair();
        for ch in pair.challenge.chars() {
            assert!(
                ch.is_ascii_alphanumeric() || ch == '-' || ch == '_',
                "invalid base64url char: {ch}"
            );
        }
    }

    #[test]
    fn pkce_pairs_differ() {
        let a = generate_pkce_pair();
        let b = generate_pkce_pair();
        assert_ne!(a.verifier, b.verifier);
    }

    #[test]
    fn state_is_32_hex_chars() {
        let s = generate_state();
        assert_eq!(s.len(), 32);
        assert!(s.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn build_url_contains_required_params() {
        let cfg = OAuthConfig {
            client_id: "my-client".into(),
            redirect_uri: "http://localhost:8080/callback".into(),
            auth_url: "https://auth.example.com/authorize".into(),
            token_url: "https://auth.example.com/token".into(),
            scopes: vec!["openid".into(), "profile".into()],
        };
        let pkce = generate_pkce_pair();
        let state = generate_state();
        let url = build_authorization_url(&cfg, &pkce, &state);
        assert!(url.starts_with("https://auth.example.com/authorize?"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("client_id=my-client"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains(&format!("state={state}")));
    }

    #[test]
    fn token_not_expired_when_no_expiry() {
        let tok = OAuthTokenSet {
            access_token: "abc".into(),
            refresh_token: None,
            expires_at: None,
            token_type: "Bearer".into(),
        };
        assert!(!token_is_expired(&tok));
    }

    #[test]
    fn token_expired_in_past() {
        let tok = OAuthTokenSet {
            access_token: "abc".into(),
            refresh_token: None,
            expires_at: Some(1),
            token_type: "Bearer".into(),
        };
        assert!(token_is_expired(&tok));
    }

    #[test]
    fn token_not_expired_in_future() {
        let future = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;
        let tok = OAuthTokenSet {
            access_token: "abc".into(),
            refresh_token: None,
            expires_at: Some(future),
            token_type: "Bearer".into(),
        };
        assert!(!token_is_expired(&tok));
    }

    #[test]
    fn persist_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tokens.json");
        let path_str = path.to_str().unwrap();

        let tokens = OAuthTokenSet {
            access_token: "access-123".into(),
            refresh_token: Some("refresh-456".into()),
            expires_at: Some(9_999_999_999),
            token_type: "Bearer".into(),
        };
        persist_tokens(path_str, &tokens).unwrap();
        let loaded = load_tokens(path_str).unwrap().unwrap();
        assert_eq!(loaded.access_token, "access-123");
        assert_eq!(loaded.refresh_token.as_deref(), Some("refresh-456"));
    }

    #[test]
    fn load_missing_file_returns_none() {
        let result = load_tokens("/tmp/nocode_oauth_nonexistent_file.json").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn sha256_known_vector() {
        // SHA-256("abc") = ba7816bf...
        let digest = sha256(b"abc");
        assert_eq!(
            digest,
            [
                0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d,
                0xae, 0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10,
                0xff, 0x61, 0xf2, 0x00, 0x15, 0xad,
            ]
        );
    }
}
