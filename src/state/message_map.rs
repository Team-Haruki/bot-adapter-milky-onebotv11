use std::num::NonZeroUsize;
use std::sync::Mutex;

use lru::LruCache;

use crate::types::MessageRef;

pub struct MessageMap {
    inner: Mutex<LruCache<i64, MessageRef>>,
}

impl MessageMap {
    pub fn new(cache_size: usize) -> Self {
        let capacity =
            NonZeroUsize::new(cache_size.max(1)).expect("cache_size guarded above zero by .max(1)");
        Self {
            inner: Mutex::new(LruCache::new(capacity)),
        }
    }

    pub fn put(&self, ref_: MessageRef) {
        let mut guard = self.inner.lock().expect("MessageMap mutex poisoned");
        guard.put(ref_.onebot_id, ref_);
    }

    pub fn get(&self, onebot_id: i64) -> Option<MessageRef> {
        let mut guard = self.inner.lock().expect("MessageMap mutex poisoned");
        guard.get(&onebot_id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ref_with_id(id: i64) -> MessageRef {
        MessageRef {
            onebot_id: id,
            milky_seq: id,
            message_type: "private".to_string(),
            user_id: 1,
            ..MessageRef::default()
        }
    }

    #[test]
    fn put_then_get_roundtrips() {
        let map = MessageMap::new(8);
        map.put(ref_with_id(42));
        let got = map.get(42).expect("present");
        assert_eq!(got.onebot_id, 42);
        assert_eq!(got.milky_seq, 42);
    }

    #[test]
    fn missing_id_returns_none() {
        let map = MessageMap::new(8);
        assert!(map.get(99).is_none());
    }

    #[test]
    fn evicts_when_capacity_exceeded() {
        let map = MessageMap::new(2);
        map.put(ref_with_id(1));
        map.put(ref_with_id(2));
        map.put(ref_with_id(3));
        assert!(
            map.get(1).is_none(),
            "oldest entry should have been evicted"
        );
        assert!(map.get(2).is_some());
        assert!(map.get(3).is_some());
    }

    #[test]
    fn get_promotes_recency() {
        let map = MessageMap::new(2);
        map.put(ref_with_id(1));
        map.put(ref_with_id(2));
        let _ = map.get(1);
        map.put(ref_with_id(3));
        assert!(
            map.get(1).is_some(),
            "recently accessed entry should survive"
        );
        assert!(map.get(2).is_none(), "stale entry should have been evicted");
    }

    #[test]
    fn zero_cache_size_falls_back_to_one() {
        let map = MessageMap::new(0);
        map.put(ref_with_id(1));
        assert!(map.get(1).is_some());
    }
}
