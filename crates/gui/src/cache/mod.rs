//! In-memory stale-while-revalidate cache for D-Bus sourced data.
//! No SQLite, no persistence — purely in-memory.
//!
//! ## TTLs (per 09-state-flow.md)
//!
//! | Data         | TTL          | Invalidation              |
//! | ------------ | ------------ | ------------------------- |
//! | Daily usage  | 500ms        | `DailyUsageChanged`       |
//! | Policies     | 5s           | `PolicyMutated`           |
//! | Block states | signal-drive | Never stale (real-time)   |

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A cache entry with its fetch timestamp.
#[derive(Debug, Clone)]
struct CacheEntry<V> {
    value: V,
    fetched_at: Instant,
}

/// Time-based stale-while-revalidate cache.
///
/// - `get()` returns `Some(value)` if the entry exists and is fresh (within TTL).
/// - `get_stale()` returns `Some(value)` even if stale (for stale-while-revalidate).
/// - `set()` inserts/updates an entry.
/// - `invalidate()` removes a specific key.
/// - `clear()` removes all entries.
///
/// Thread-safe via `Mutex` — contention is negligible (one render thread + one
/// tokio thread).
#[derive(Debug)]
pub struct ClientCache<K: Eq + Hash + Clone, V: Clone> {
    inner: Mutex<HashMap<K, CacheEntry<V>>>,
    ttl: Duration,
}

impl<K: Eq + Hash + Clone, V: Clone> ClientCache<K, V> {
    /// Create a new cache with the given per-entry TTL.
    pub fn new(ttl: Duration) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            ttl,
        }
    }

    /// Returns cached value if fresh (elapsed < TTL), or `None` if stale/missing.
    pub fn get(&self, key: &K) -> Option<V> {
        let map = self.inner.lock().unwrap();
        match map.get(key) {
            Some(entry) if entry.fetched_at.elapsed() < self.ttl => Some(entry.value.clone()),
            _ => None,
        }
    }

    /// Returns cached value even if stale (for stale-while-revalidate pattern).
    /// Returns `None` only if the key is missing entirely.
    pub fn get_stale(&self, key: &K) -> Option<V> {
        let map = self.inner.lock().unwrap();
        map.get(key).map(|e| e.value.clone())
    }

    /// Returns true if the key exists and is fresh.
    pub fn is_fresh(&self, key: &K) -> bool {
        let map = self.inner.lock().unwrap();
        map.get(key)
            .is_some_and(|e| e.fetched_at.elapsed() < self.ttl)
    }

    /// Insert or update a cache entry (resets its fetch timestamp).
    pub fn set(&self, key: K, value: V) {
        let mut map = self.inner.lock().unwrap();
        map.insert(
            key,
            CacheEntry {
                value,
                fetched_at: Instant::now(),
            },
        );
    }

    /// Remove a specific key from the cache.
    pub fn invalidate(&self, key: &K) {
        let mut map = self.inner.lock().unwrap();
        map.remove(key);
    }

    /// Clear all entries.
    pub fn clear(&self) {
        let mut map = self.inner.lock().unwrap();
        map.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    #[test]
    fn test_get_and_set() {
        let cache: ClientCache<String, i32> = ClientCache::new(Duration::from_secs(10));
        cache.set("key1".into(), 42);
        assert_eq!(cache.get(&"key1".into()), Some(42));
    }

    #[test]
    fn test_missing_key() {
        let cache: ClientCache<String, i32> = ClientCache::new(Duration::from_secs(10));
        assert_eq!(cache.get(&"nonexistent".into()), None);
    }

    #[test]
    fn test_stale_entry() {
        let cache: ClientCache<String, i32> = ClientCache::new(Duration::from_millis(10));
        cache.set("key1".into(), 42);
        sleep(Duration::from_millis(20));
        // Fresh get should return None
        assert_eq!(cache.get(&"key1".into()), None);
        // Stale get should still return the value
        assert_eq!(cache.get_stale(&"key1".into()), Some(42));
    }

    #[test]
    fn test_invalidate() {
        let cache: ClientCache<String, i32> = ClientCache::new(Duration::from_secs(10));
        cache.set("key1".into(), 42);
        cache.invalidate(&"key1".into());
        assert_eq!(cache.get(&"key1".into()), None);
        assert_eq!(cache.get_stale(&"key1".into()), None);
    }

    #[test]
    fn test_clear() {
        let cache: ClientCache<String, i32> = ClientCache::new(Duration::from_secs(10));
        cache.set("a".into(), 1);
        cache.set("b".into(), 2);
        cache.clear();
        assert_eq!(cache.get(&"a".into()), None);
        assert_eq!(cache.get(&"b".into()), None);
    }
}
