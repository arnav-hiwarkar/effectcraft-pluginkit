use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub(crate) const IMAGE_BUDGET: usize = 256 << 20;
pub const MAX_IMAGE_SIDE: u32 = 8192;

static GENERATION: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
pub struct PluginImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub generation: u64,
}

#[derive(Default)]
struct Inner {
    images: HashMap<String, Arc<PluginImage>>,
    bytes: usize,
}

pub(crate) struct Images {
    budget: usize,
    inner: Mutex<Inner>,
}

impl Images {
    pub fn new(budget: usize) -> Self {
        Images { budget, inner: Mutex::default() }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn get(&self, key: &str) -> Option<Arc<PluginImage>> {
        self.lock().images.get(key).cloned()
    }

    pub fn put(&self, key: String, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
        if width == 0 || height == 0 || width > MAX_IMAGE_SIDE || height > MAX_IMAGE_SIDE {
            return Err(format!("an image is 1 to {MAX_IMAGE_SIDE} pixels on a side, not {width}x{height}"));
        }
        let wanted = width as usize * height as usize * 4;
        if rgba.len() != wanted {
            return Err(format!("a {width}x{height} image has {wanted} bytes, not {}", rgba.len()));
        }
        let mut inner = self.lock();
        let replaced = inner.images.get(&key).map_or(0, |old| old.rgba.len());
        let total = inner.bytes - replaced + wanted;
        if total > self.budget {
            return Err(format!("the plugin's images would take {total} bytes, more than the {} allowed", self.budget));
        }
        let image = PluginImage { width, height, rgba: rgba.to_vec(), generation: GENERATION.fetch_add(1, Ordering::Relaxed) };
        inner.images.insert(key, Arc::new(image));
        inner.bytes = total;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Images;

    #[test]
    fn the_same_key_replaces_and_the_budget_counts_the_replacement_once() {
        let images = Images::new(32);
        assert_eq!(images.put("a".into(), 2, 2, &[1; 16]), Ok(()));
        let first = images.get("a").map(|i| i.generation);
        assert_eq!(images.put("a".into(), 2, 2, &[2; 16]), Ok(()));
        assert_eq!(images.put("b".into(), 2, 2, &[3; 16]), Ok(()));
        let replaced = images.get("a");
        assert_eq!(replaced.as_ref().map(|i| i.rgba[0]), Some(2));
        assert_ne!(replaced.map(|i| i.generation), first);
        assert!(images.put("c".into(), 1, 1, &[0; 4]).is_err_and(|e| e.contains("more than the 32 allowed")));
    }

    #[test]
    fn an_image_larger_than_the_side_limit_or_with_the_wrong_byte_count_is_refused() {
        let images = Images::new(usize::MAX);
        assert!(images.put("wide".into(), 8193, 1, &[0; 8193 * 4]).is_err());
        assert!(images.put("short".into(), 2, 2, &[0; 15]).is_err());
        assert!(images.get("wide").is_none());
    }
}
