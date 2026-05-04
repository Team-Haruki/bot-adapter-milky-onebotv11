use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ApiRequest {
    #[serde(default)]
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub echo: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApiResponse {
    pub status: &'static str,
    pub retcode: i32,
    pub data: Value,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub msg: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub wording: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub echo: Option<Value>,
}

pub fn normalize_action(action: &str) -> String {
    let trimmed = action.trim();
    let trimmed = trimmed.strip_suffix("_async").unwrap_or(trimmed);
    let trimmed = trimmed.strip_suffix("_rate_limited").unwrap_or(trimmed);
    trimmed.to_string()
}

pub fn success(data: Value, echo: Option<Value>) -> ApiResponse {
    ApiResponse {
        status: "ok",
        retcode: 0,
        data,
        msg: String::new(),
        wording: String::new(),
        echo,
    }
}

pub fn failure(retcode: i32, message: impl Into<String>, echo: Option<Value>) -> ApiResponse {
    let m = message.into();
    ApiResponse {
        status: "failed",
        retcode,
        data: Value::Null,
        msg: m.clone(),
        wording: m,
        echo,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_suffixes() {
        assert_eq!(normalize_action("send_msg"), "send_msg");
        assert_eq!(normalize_action("send_msg_async"), "send_msg");
        assert_eq!(normalize_action("send_msg_rate_limited"), "send_msg");
        assert_eq!(normalize_action("  send_msg  "), "send_msg");
    }

    #[test]
    fn success_serializes_ok() {
        let resp = success(serde_json::json!({"x": 1}), Some(Value::String("e".into())));
        let s = serde_json::to_string(&resp).unwrap();
        assert!(s.contains("\"status\":\"ok\""));
        assert!(s.contains("\"retcode\":0"));
        assert!(s.contains("\"x\":1"));
        assert!(s.contains("\"echo\":\"e\""));
    }

    #[test]
    fn failure_serializes_failed_with_msg() {
        let resp = failure(1400, "bad params", None);
        let s = serde_json::to_string(&resp).unwrap();
        assert!(s.contains("\"status\":\"failed\""));
        assert!(s.contains("\"retcode\":1400"));
        assert!(s.contains("\"msg\":\"bad params\""));
        assert!(s.contains("\"wording\":\"bad params\""));
        assert!(!s.contains("\"echo\""));
    }
}
