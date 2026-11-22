use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub(crate) const BUDGET: usize = 1 << 30;

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
        if bytes.len() > self.budget {
            return Err(format!("{} bytes is more than the plugin cache holds", bytes.len()));
        }
        let mut inner = self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.clock += 1;
        let used = inner.clock;
        if let Some(old) = inner.entries.insert(key, Entry { bytes: bytes.into(), used }) {
            inner.bytes -= old.bytes.len();
        }
        inner.bytes += bytes.len();
        while inner.bytes > self.budget {
            let Some(oldest) = inner.entries.iter().min_by_key(|(_, entry)| entry.used).map(|(key, _)| key.clone()) else { break };
            if let Some(evicted) = inner.entries.remove(&oldest) {
                inner.bytes -= evicted.bytes.len();
            }
        }
        Ok(())
    }

    pub fn remove(&self, key: &str) {
        let mut inner = self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(removed) = inner.entries.remove(key) {
            inner.bytes -= removed.bytes.len();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Cache;

    #[test]
    fn the_least_recently_used_entry_leaves_first() {
        let cache = Cache::new(10);
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
        let cache = Cache::new(3);
        assert_eq!(cache.put("a".into(), &[0; 4]), Err("4 bytes is more than the plugin cache holds".into()));
    }

    #[test]
    fn replacing_an_entry_frees_its_old_bytes() {
        let cache = Cache::new(8);
        cache.put("a".into(), &[1; 8]).unwrap();
        cache.put("a".into(), &[2; 8]).unwrap();
        assert_eq!(cache.get("a").as_deref(), Some(&[2u8; 8][..]));
    }
}
