//! In-memory write-through cache for D-Bus sourced data.
//! No SQLite, no persistence, no expiry — purely in-memory.
//!
//! ## Design
//!
//! `ClientCache` is a simple key/value store used to deduplicate D-Bus calls
//! within a single refresh cycle. Callers explicitly invalidate the cache
//! (via `invalidate()` or `clear()`) when they know the data has changed —
//! the cache itself never expires entries on its own.
//!
//! | Data         | Invalidation trigger          |
//! | ------------ | ----------------------------- |
//! | Daily usage  | `DailyUsageChanged` signal    |
//! | Policies     | `PolicyMutated` signal        |
//! | Categories   | `PolicyMutated` signal        |
//! | Block states | `BlockStateChanged` signal    |

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Mutex;

/// Simple in-memory cache with no time-based expiry.
///
/// Thread-safe via `Mutex` — contention is negligible (one render thread +
/// one tokio thread).
#[derive(Debug)]
pub struct ClientCache<K: Eq + Hash + Clone, V: Clone> {
    inner: Mutex<HashMap<K, V>>,
}

impl<K: Eq + Hash + Clone, V: Clone> Default for ClientCache<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Eq + Hash + Clone, V: Clone> ClientCache<K, V> {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    pub fn get(&self, key: &K) -> Option<V> {
        self.inner.lock().unwrap().get(key).cloned()
    }

    pub fn set(&self, key: K, value: V) {
        self.inner.lock().unwrap().insert(key, value);
    }

    pub fn invalidate(&self, key: &K) {
        self.inner.lock().unwrap().remove(key);
    }

    pub fn clear(&self) {
        self.inner.lock().unwrap().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_and_set() {
        let cache: ClientCache<String, i32> = ClientCache::new();
        cache.set("key1".into(), 42);
        assert_eq!(cache.get(&"key1".into()), Some(42));
    }

    #[test]
    fn test_missing_key() {
        let cache: ClientCache<String, i32> = ClientCache::new();
        assert_eq!(cache.get(&"nonexistent".into()), None);
    }

    #[test]
    fn test_invalidate() {
        let cache: ClientCache<String, i32> = ClientCache::new();
        cache.set("key1".into(), 42);
        cache.invalidate(&"key1".into());
        assert_eq!(cache.get(&"key1".into()), None);
    }

    #[test]
    fn test_clear() {
        let cache: ClientCache<String, i32> = ClientCache::new();
        cache.set("a".into(), 1);
        cache.set("b".into(), 2);
        cache.clear();
        assert_eq!(cache.get(&"a".into()), None);
        assert_eq!(cache.get(&"b".into()), None);
    }
}
