use std::collections::HashMap;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::types::RequestRef;

pub struct RequestMap {
    counter: AtomicU64,
    items: RwLock<HashMap<String, RequestRef>>,
}

impl RequestMap {
    pub fn new() -> Self {
        Self {
            counter: AtomicU64::new(0),
            items: RwLock::new(HashMap::new()),
        }
    }

    pub fn put(&self, ref_: RequestRef) -> String {
        let n = self.counter.fetch_add(1, Ordering::Relaxed) + 1;
        let flag = format!("{}:{}", ref_.kind, n);
        self.items
            .write()
            .expect("RequestMap rwlock poisoned")
            .insert(flag.clone(), ref_);
        flag
    }

    pub fn get(&self, flag: &str) -> Option<RequestRef> {
        self.items
            .read()
            .expect("RequestMap rwlock poisoned")
            .get(flag)
            .cloned()
    }
}

impl Default for RequestMap {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_returns_unique_flag_with_kind_prefix() {
        let map = RequestMap::new();
        let a = map.put(RequestRef {
            kind: "friend".to_string(),
            initiator_uid: 42,
            ..RequestRef::default()
        });
        let b = map.put(RequestRef {
            kind: "group".to_string(),
            group_id: 100,
            ..RequestRef::default()
        });
        assert_eq!(a, "friend:1");
        assert_eq!(b, "group:2");
    }

    #[test]
    fn get_returns_stored_ref() {
        let map = RequestMap::new();
        let flag = map.put(RequestRef {
            kind: "group".to_string(),
            group_id: 555,
            invitation_seq: 7,
            ..RequestRef::default()
        });
        let got = map.get(&flag).expect("present");
        assert_eq!(got.group_id, 555);
        assert_eq!(got.invitation_seq, 7);
    }

    #[test]
    fn missing_flag_returns_none() {
        let map = RequestMap::new();
        assert!(map.get("ghost:1").is_none());
    }
}
