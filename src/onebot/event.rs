use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

pub fn lifecycle_event(self_id: i64, sub_type: &str) -> Value {
    json!({
        "time": now_unix(),
        "self_id": self_id,
        "post_type": "meta_event",
        "meta_event_type": "lifecycle",
        "sub_type": sub_type,
    })
}

pub fn heartbeat_event(self_id: i64, status: Value, interval_ms: u64) -> Value {
    json!({
        "time": now_unix(),
        "self_id": self_id,
        "post_type": "meta_event",
        "meta_event_type": "heartbeat",
        "status": status,
        "interval": interval_ms,
    })
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_shape() {
        let v = lifecycle_event(42, "connect");
        assert_eq!(v["self_id"], 42);
        assert_eq!(v["post_type"], "meta_event");
        assert_eq!(v["meta_event_type"], "lifecycle");
        assert_eq!(v["sub_type"], "connect");
        assert!(v["time"].is_i64());
    }

    #[test]
    fn heartbeat_shape() {
        let status = json!({"online": true, "good": true});
        let v = heartbeat_event(7, status, 15000);
        assert_eq!(v["meta_event_type"], "heartbeat");
        assert_eq!(v["interval"], 15000);
        assert_eq!(v["status"]["online"], true);
    }
}
