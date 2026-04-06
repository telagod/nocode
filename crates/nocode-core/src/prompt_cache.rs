use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// Returns current time in milliseconds since UNIX epoch.
#[must_use]
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// FNV-1a 64-bit hash.
#[must_use]
pub fn fnv1a_hash(data: &[u8]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0100_0000_01b3;
    let mut hash = FNV_OFFSET;
    for &byte in data {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Compute a deterministic fingerprint from request components.
#[must_use]
pub fn compute_fingerprint(
    model: &str,
    system_prompt: &str,
    messages: &[(&str, &str)],
    tools: &[&str],
) -> u64 {
    let mut buf = Vec::with_capacity(256);
    buf.extend_from_slice(model.as_bytes());
    buf.push(0xff);
    buf.extend_from_slice(system_prompt.as_bytes());
    buf.push(0xff);
    for (role, content) in messages {
        buf.extend_from_slice(role.as_bytes());
        buf.push(0xfe);
        buf.extend_from_slice(content.as_bytes());
        buf.push(0xfe);
    }
    buf.push(0xff);
    for tool in tools {
        buf.extend_from_slice(tool.as_bytes());
        buf.push(0xfd);
    }
    fnv1a_hash(&buf)
}

/// A single cached prompt/response pair.
#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub fingerprint: u64,
    pub response: String,
    pub created_at_ms: u64,
    pub ttl_ms: u64,
    pub hit_count: u32,
}

/// Aggregate cache statistics.
#[derive(Debug, Clone, Default)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub total_saved_tokens: u64,
}

/// In-memory prompt cache with TTL and LRU-by-age eviction.
pub struct PromptCache {
    entries: HashMap<u64, CacheEntry>,
    stats: CacheStats,
    max_entries: usize,
    default_ttl_ms: u64,
}

impl Default for PromptCache {
    fn default() -> Self {
        Self::new(256, 30_000)
    }
}

impl PromptCache {
    /// Create a new cache with the given capacity and default TTL.
    #[must_use]
    pub fn new(max_entries: usize, default_ttl_ms: u64) -> Self {
        Self {
            entries: HashMap::with_capacity(max_entries),
            stats: CacheStats::default(),
            max_entries,
            default_ttl_ms,
        }
    }

    /// Look up a cached entry. Returns `None` if missing or expired.
    pub fn get(&mut self, fingerprint: u64) -> Option<&CacheEntry> {
        let now = now_ms();
        // Check expiry first — remove if stale.
        if let Some(entry) = self.entries.get(&fingerprint)
            && now.saturating_sub(entry.created_at_ms) > entry.ttl_ms
        {
            self.entries.remove(&fingerprint);
            self.stats.evictions += 1;
            self.stats.misses += 1;
            return None;
        }
        if self.entries.contains_key(&fingerprint) {
            self.stats.hits += 1;
            // Bump hit count via get_mut, then return shared ref.
            if let Some(e) = self.entries.get_mut(&fingerprint) {
                e.hit_count += 1;
            }
            self.entries.get(&fingerprint)
        } else {
            self.stats.misses += 1;
            None
        }
    }

    /// Insert or update a cached response. Evicts the oldest entry when full.
    pub fn put(&mut self, fingerprint: u64, response: String, ttl_ms: Option<u64>) {
        let ttl = ttl_ms.unwrap_or(self.default_ttl_ms);
        // If already present, just overwrite.
        if self.entries.contains_key(&fingerprint) {
            if let Some(e) = self.entries.get_mut(&fingerprint) {
                e.response = response;
                e.created_at_ms = now_ms();
                e.ttl_ms = ttl;
                e.hit_count = 0;
            }
            return;
        }
        // Evict oldest if at capacity.
        if self.entries.len() >= self.max_entries {
            self.evict_oldest();
        }
        self.entries.insert(
            fingerprint,
            CacheEntry {
                fingerprint,
                response,
                created_at_ms: now_ms(),
                ttl_ms: ttl,
                hit_count: 0,
            },
        );
    }

    /// Remove a specific entry. Returns `true` if it existed.
    pub fn invalidate(&mut self, fingerprint: u64) -> bool {
        self.entries.remove(&fingerprint).is_some()
    }

    /// Drop all entries and reset stats.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.stats = CacheStats::default();
    }

    /// Current cache statistics.
    #[must_use]
    pub fn stats(&self) -> &CacheStats {
        &self.stats
    }

    /// Remove all expired entries. Returns the number evicted.
    pub fn evict_expired(&mut self) -> usize {
        let now = now_ms();
        let before = self.entries.len();
        self.entries
            .retain(|_, e| now.saturating_sub(e.created_at_ms) <= e.ttl_ms);
        let evicted = before - self.entries.len();
        self.stats.evictions += evicted as u64;
        evicted
    }

    /// Evict the single oldest entry by `created_at_ms`.
    fn evict_oldest(&mut self) {
        if let Some(&key) = self
            .entries
            .values()
            .min_by_key(|e| e.created_at_ms)
            .map(|e| &e.fingerprint)
        {
            self.entries.remove(&key);
            self.stats.evictions += 1;
        }
    }

    /// Test helper: manually backdate an entry so it appears expired.
    #[cfg(test)]
    fn backdate(&mut self, fingerprint: u64, shift_ms: u64) {
        if let Some(e) = self.entries.get_mut(&fingerprint) {
            e.created_at_ms = e.created_at_ms.saturating_sub(shift_ms);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_and_get_within_ttl() {
        let mut cache = PromptCache::new(16, 60_000);
        let fp = compute_fingerprint("m", "sys", &[("user", "hi")], &["tool1"]);
        cache.put(fp, "response-1".into(), None);
        let entry = cache.get(fp).expect("should hit");
        assert_eq!(entry.response, "response-1");
        assert_eq!(entry.hit_count, 1);
    }

    #[test]
    fn get_expired_returns_none() {
        let mut cache = PromptCache::new(16, 1_000);
        let fp = 42;
        cache.put(fp, "old".into(), Some(1_000));
        // Backdate so it looks expired.
        cache.backdate(fp, 2_000);
        assert!(cache.get(fp).is_none());
        assert_eq!(cache.stats().misses, 1);
        assert_eq!(cache.stats().evictions, 1);
    }

    #[test]
    fn evict_oldest_when_full() {
        let mut cache = PromptCache::new(2, 60_000);
        cache.put(1, "a".into(), None);
        // Backdate entry 1 so it is the oldest.
        cache.backdate(1, 10_000);
        cache.put(2, "b".into(), None);
        // Cache is full (2). Inserting a third should evict the oldest (1).
        cache.put(3, "c".into(), None);
        assert!(cache.get(1).is_none()); // evicted
        assert!(cache.get(2).is_some());
        assert!(cache.get(3).is_some());
        assert!(cache.stats().evictions >= 1);
    }

    #[test]
    fn invalidate_removes_entry() {
        let mut cache = PromptCache::default();
        cache.put(99, "x".into(), None);
        assert!(cache.invalidate(99));
        assert!(!cache.invalidate(99)); // already gone
        assert!(cache.get(99).is_none());
    }

    #[test]
    fn clear_resets_all() {
        let mut cache = PromptCache::default();
        cache.put(1, "a".into(), None);
        cache.put(2, "b".into(), None);
        let _ = cache.get(1);
        cache.clear();
        assert!(cache.get(1).is_none());
        assert!(cache.get(2).is_none());
        assert_eq!(cache.stats().hits, 0);
        assert_eq!(cache.stats().evictions, 0);
    }

    #[test]
    fn stats_track_hits_and_misses() {
        let mut cache = PromptCache::new(16, 60_000);
        cache.put(1, "a".into(), None);
        let _ = cache.get(1); // hit
        let _ = cache.get(1); // hit
        let _ = cache.get(999); // miss
        assert_eq!(cache.stats().hits, 2);
        assert_eq!(cache.stats().misses, 1);
    }

    #[test]
    fn fingerprint_deterministic() {
        let a = compute_fingerprint("m", "s", &[("u", "hi")], &["t"]);
        let b = compute_fingerprint("m", "s", &[("u", "hi")], &["t"]);
        assert_eq!(a, b);
    }

    #[test]
    fn fingerprint_differs_for_different_input() {
        let a = compute_fingerprint("m1", "s", &[], &[]);
        let b = compute_fingerprint("m2", "s", &[], &[]);
        assert_ne!(a, b);

        let c = compute_fingerprint("m", "s1", &[], &[]);
        let d = compute_fingerprint("m", "s2", &[], &[]);
        assert_ne!(c, d);
    }

    #[test]
    fn evict_expired_cleans_stale() {
        let mut cache = PromptCache::new(16, 5_000);
        cache.put(1, "a".into(), Some(5_000));
        cache.put(2, "b".into(), Some(5_000));
        cache.put(3, "c".into(), Some(60_000));
        // Backdate 1 and 2 so they are expired.
        cache.backdate(1, 10_000);
        cache.backdate(2, 10_000);
        let evicted = cache.evict_expired();
        assert_eq!(evicted, 2);
        assert!(cache.get(1).is_none());
        assert!(cache.get(2).is_none());
        assert!(cache.get(3).is_some());
    }
}
