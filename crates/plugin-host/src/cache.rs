use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub(crate) const BUDGET: usize = 1 << 30;
const ENTRY_OVERHEAD: usize = 64;
const KEY_LIMIT: usize = 1024;

fn charge(key: &str, bytes: &[u8]) -> usize {
    key.len() + bytes.len() + ENTRY_OVERHEAD
}

struct Entry {
    bytes: Arc<[u8]>,
    used: u64,
}

#[derive(Default)]
struct Inner {
    entries: HashMap<String, Entry>,
    bytes: usize,
    clock: u64,
}

pub(crate) struct Cache {
    budget: usize,
    inner: Mutex<Inner>,
}

impl Cache {
    pub fn new(budget: usize) -> Self {
        Cache { budget, inner: Mutex::default() }
    }

    pub fn get(&self, key: &str) -> Option<Arc<[u8]>> {
        let mut inner = self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.clock += 1;
        let clock = inner.clock;
        let entry = inner.entries.get_mut(key)?;
        entry.used = clock;
        Some(entry.bytes.clone())
    }

    pub fn put(&self, key: String, bytes: &[u8]) -> Result<(), String> {
        if key.len() > KEY_LIMIT {
            return Err(format!("a cache key of {} bytes is longer than {KEY_LIMIT}", key.len()));
        }
        let cost = charge(&key, bytes);
        if cost > self.budget {
            return Err(format!("{} bytes is more than the plugin cache holds", bytes.len()));
        }
        let mut inner = self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.clock += 1;
        let used = inner.clock;
        if let Some(old) = inner.entries.remove(&key) {
            inner.bytes -= charge(&key, &old.bytes);
        }
        inner.entries.insert(key, Entry { bytes: bytes.into(), used });
        inner.bytes += cost;
        while inner.bytes > self.budget {
            let Some(oldest) = inner.entries.iter().min_by_key(|(_, entry)| entry.used).map(|(key, _)| key.clone()) else { break };
            if let Some(evicted) = inner.entries.remove(&oldest) {
                inner.bytes -= charge(&oldest, &evicted.bytes);
            }
        }
        Ok(())
    }

    pub fn remove(&self, key: &str) {
        let mut inner = self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(removed) = inner.entries.remove(key) {
            inner.bytes -= charge(key, &removed.bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Cache;

    #[test]
    fn the_least_recently_used_entry_leaves_first() {
        let cache = Cache::new(10 + 2 * 65);
        cache.put("a".into(), &[1; 4]).unwrap();
        cache.put("b".into(), &[2; 4]).unwrap();
        assert_eq!(cache.get("a").as_deref(), Some(&[1u8; 4][..]));
        cache.put("c".into(), &[3; 4]).unwrap();
        assert_eq!(cache.get("b"), None);
        assert_eq!(cache.get("a").as_deref(), Some(&[1u8; 4][..]));
        assert_eq!(cache.get("c").as_deref(), Some(&[3u8; 4][..]));
    }

    #[test]
    fn an_entry_larger_than_the_budget_is_refused() {
        let cache = Cache::new(68);
        assert_eq!(cache.put("a".into(), &[0; 4]), Err("4 bytes is more than the plugin cache holds".into()));
    }

    #[test]
    fn replacing_an_entry_frees_its_old_bytes() {
        let cache = Cache::new(8 + 65);
        cache.put("a".into(), &[1; 8]).unwrap();
        cache.put("a".into(), &[2; 8]).unwrap();
        assert_eq!(cache.get("a").as_deref(), Some(&[2u8; 8][..]));
    }

    #[test]
    fn empty_values_still_use_the_budget() {
        let cache = Cache::new(2 * 66);
        for key in ["k1", "k2", "k3"] {
            cache.put(key.into(), &[]).unwrap();
        }
        assert_eq!(cache.get("k1"), None);
        assert_eq!(cache.get("k2").as_deref(), Some(&[][..]));
        assert_eq!(cache.get("k3").as_deref(), Some(&[][..]));
    }

    #[test]
    fn a_key_longer_than_1024_bytes_is_refused() {
        let cache = Cache::new(1 << 20);
        assert_eq!(cache.put("k".repeat(1025), &[1]), Err("a cache key of 1025 bytes is longer than 1024".into()));
        assert_eq!(cache.put("k".repeat(1024), &[1]), Ok(()));
    }
}
