use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("read config: {0}")]
    Io(#[from] std::io::Error),
    #[error("decode config: {0}")]
    Decode(#[from] serde_json::Error),
    #[error("invalid config: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub milky: MilkyConfig,
    #[serde(default)]
    pub onebot: OneBotConfig,
    #[serde(default)]
    pub bridge: BridgeConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MilkyConfig {
    #[serde(default)]
    pub ws_endpoint: String,
    #[serde(default)]
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OneBotConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub access_token: String,
    #[serde(default)]
    pub enable_http_api: bool,
    #[serde(default = "default_true")]
    pub enable_ws_api: bool,
    #[serde(default = "default_true")]
    pub enable_ws_event: bool,
    #[serde(default = "default_true")]
    pub enable_ws_universal: bool,
    #[serde(default)]
    pub reverse: OneBotReverseConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OneBotReverseConfig {
    #[serde(default)]
    pub enable: bool,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub api_url: String,
    #[serde(default)]
    pub event_url: String,
    #[serde(default)]
    pub use_universal_client: bool,
    #[serde(default = "default_reconnect_interval_ms")]
    pub reconnect_interval_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BridgeConfig {
    #[serde(default)]
    pub self_id: i64,
    #[serde(default = "default_message_format")]
    pub message_format: String,
    #[serde(default = "default_heartbeat_interval_ms")]
    pub heartbeat_interval_ms: u64,
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default = "default_cache_size")]
    pub cache_size: usize,
}

impl Default for OneBotConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            access_token: String::new(),
            enable_http_api: false,
            enable_ws_api: true,
            enable_ws_event: true,
            enable_ws_universal: true,
            reverse: OneBotReverseConfig::default(),
        }
    }
}

impl Default for OneBotReverseConfig {
    fn default() -> Self {
        Self {
            enable: false,
            url: String::new(),
            api_url: String::new(),
            event_url: String::new(),
            use_universal_client: false,
            reconnect_interval_ms: default_reconnect_interval_ms(),
        }
    }
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            self_id: 0,
            message_format: default_message_format(),
            heartbeat_interval_ms: default_heartbeat_interval_ms(),
            log_level: default_log_level(),
            cache_size: default_cache_size(),
        }
    }
}

fn default_host() -> String {
    "0.0.0.0".to_string()
}
fn default_port() -> u16 {
    6700
}
fn default_true() -> bool {
    true
}
fn default_reconnect_interval_ms() -> u64 {
    3000
}
fn default_message_format() -> String {
    "array".to_string()
}
fn default_heartbeat_interval_ms() -> u64 {
    15000
}
fn default_log_level() -> String {
    "info".to_string()
}
fn default_cache_size() -> usize {
    2048
}

impl Config {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let data = std::fs::read_to_string(path)?;
        let cfg: Self = serde_json::from_str(&data)?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        let invalid = |msg: &str| ConfigError::Invalid(msg.to_string());

        if self.milky.ws_endpoint.trim().is_empty() {
            return Err(invalid("milky.ws_endpoint is required"));
        }
        if self.onebot.port == 0 {
            return Err(invalid("onebot.port must be between 1 and 65535"));
        }
        if self.onebot.host.trim().is_empty() {
            return Err(invalid("onebot.host is required"));
        }
        if self.bridge.message_format != "array" && self.bridge.message_format != "string" {
            return Err(invalid("bridge.message_format must be array or string"));
        }
        if self.bridge.heartbeat_interval_ms == 0 {
            return Err(invalid("bridge.heartbeat_interval_ms must be positive"));
        }
        if self.bridge.cache_size == 0 {
            return Err(invalid("bridge.cache_size must be positive"));
        }
        if self.onebot.reverse.enable && self.onebot.reverse.reconnect_interval_ms == 0 {
            return Err(invalid(
                "onebot.reverse.reconnect_interval_ms must be positive",
            ));
        }
        if self.onebot.reverse.enable {
            let url = self.onebot.reverse.url.trim();
            let api = self.onebot.reverse.api_url.trim();
            let event = self.onebot.reverse.event_url.trim();
            if url.is_empty() && api.is_empty() && event.is_empty() {
                let msg = if self.onebot.reverse.use_universal_client {
                    "onebot.reverse requires url when universal reverse ws is enabled"
                } else {
                    "onebot.reverse requires url/api_url/event_url when reverse ws is enabled"
                };
                return Err(invalid(msg));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn load_full_config() {
        let mut file = NamedTempFile::new().expect("tempfile");
        let body = r#"{
  "milky": {
    "ws_endpoint": "ws://127.0.0.1:22345",
    "token": ""
  },
  "onebot": {
    "host": "127.0.0.1",
    "port": 6700,
    "access_token": "",
    "enable_http_api": false,
    "enable_ws_api": true,
    "enable_ws_event": true,
    "enable_ws_universal": true,
    "reverse": {
      "enable": false,
      "url": "",
      "api_url": "",
      "event_url": "",
      "use_universal_client": false,
      "reconnect_interval_ms": 3000
    }
  },
  "bridge": {
    "self_id": 123,
    "message_format": "array",
    "heartbeat_interval_ms": 15000,
    "log_level": "info",
    "cache_size": 128
  }
}"#;
        file.write_all(body.as_bytes()).unwrap();

        let cfg = Config::load(file.path()).expect("load");
        assert_eq!(cfg.milky.ws_endpoint, "ws://127.0.0.1:22345");
        assert_eq!(cfg.onebot.port, 6700);
        assert_eq!(cfg.bridge.self_id, 123);
        assert_eq!(cfg.bridge.cache_size, 128);
    }

    #[test]
    fn defaults_apply_when_fields_missing() {
        let mut file = NamedTempFile::new().unwrap();
        let body = r#"{
  "milky": {
    "ws_endpoint": "ws://127.0.0.1:22345"
  }
}"#;
        file.write_all(body.as_bytes()).unwrap();
        let cfg = Config::load(file.path()).expect("load");
        assert_eq!(cfg.onebot.host, "0.0.0.0");
        assert_eq!(cfg.onebot.port, 6700);
        assert!(cfg.onebot.enable_ws_api);
        assert_eq!(cfg.bridge.message_format, "array");
        assert_eq!(cfg.bridge.heartbeat_interval_ms, 15000);
    }

    #[test]
    fn unknown_fields_rejected() {
        let mut file = NamedTempFile::new().unwrap();
        let body = r#"{
  "milky": {"ws_endpoint": "ws://x"},
  "extra": 1
}"#;
        file.write_all(body.as_bytes()).unwrap();
        let err = Config::load(file.path()).unwrap_err();
        assert!(matches!(err, ConfigError::Decode(_)));
    }

    #[test]
    fn missing_ws_endpoint_rejected() {
        let mut file = NamedTempFile::new().unwrap();
        let body = r#"{"milky": {}}"#;
        file.write_all(body.as_bytes()).unwrap();
        let err = Config::load(file.path()).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid(_)));
    }

    #[test]
    fn reverse_ws_requires_url() {
        let mut file = NamedTempFile::new().unwrap();
        let body = r#"{
  "milky": {"ws_endpoint": "ws://x"},
  "onebot": {"reverse": {"enable": true}}
}"#;
        file.write_all(body.as_bytes()).unwrap();
        let err = Config::load(file.path()).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid(_)));
    }
}
