use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;
use tokio::sync::{mpsc, watch};
use tokio::time::{MissedTickBehavior, interval};

use super::message_ir::parse_onebot_message;
use super::translator::{build_get_msg_sender, translate_event, unsupported_action};
use crate::config::Config;
use crate::milky::{Client as MilkyClient, MilkyClientError};
use crate::onebot::{
    ApiRequest, ApiResponse, Handler, Server, failure, heartbeat_event, lifecycle_event,
    normalize_action, success,
};
use crate::state::{MessageMap, RequestMap, Runtime};
use crate::types::{
    GroupInfo, GroupMemberInfo, InboundEvent, LoginInfo, MessageRef, RequestRef, Segment,
};

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error("upstream: {0}")]
    Upstream(#[from] MilkyClientError),
}

#[async_trait]
pub trait Upstream: Send + Sync + 'static {
    async fn send_private_message(
        &self,
        user_id: i64,
        segments: Vec<Segment>,
    ) -> Result<i64, MilkyClientError>;
    async fn send_group_message(
        &self,
        group_id: i64,
        segments: Vec<Segment>,
    ) -> Result<i64, MilkyClientError>;
    async fn get_group_info(&self, group_id: i64) -> Result<GroupInfo, MilkyClientError>;
    async fn get_group_list(&self) -> Result<Vec<GroupInfo>, MilkyClientError>;
    async fn get_group_member_info(
        &self,
        group_id: i64,
        user_id: i64,
    ) -> Result<GroupMemberInfo, MilkyClientError>;
    async fn get_group_member_list(
        &self,
        group_id: i64,
    ) -> Result<Vec<GroupMemberInfo>, MilkyClientError>;
    async fn get_message(&self, message_ref: &MessageRef)
    -> Result<InboundEvent, MilkyClientError>;
    async fn delete_message(&self, message_ref: &MessageRef) -> Result<(), MilkyClientError>;
    async fn handle_friend_request(
        &self,
        request: &RequestRef,
        approve: bool,
        reason: String,
    ) -> Result<(), MilkyClientError>;
    async fn handle_group_request(
        &self,
        request: &RequestRef,
        approve: bool,
    ) -> Result<(), MilkyClientError>;
    async fn connect(&self) -> Result<LoginInfo, MilkyClientError>;
    async fn shutdown(&self);
}

#[async_trait]
impl Upstream for MilkyClient {
    async fn send_private_message(
        &self,
        user_id: i64,
        segments: Vec<Segment>,
    ) -> Result<i64, MilkyClientError> {
        MilkyClient::send_private_message(self, user_id, segments).await
    }
    async fn send_group_message(
        &self,
        group_id: i64,
        segments: Vec<Segment>,
    ) -> Result<i64, MilkyClientError> {
        MilkyClient::send_group_message(self, group_id, segments).await
    }
    async fn get_group_info(&self, group_id: i64) -> Result<GroupInfo, MilkyClientError> {
        MilkyClient::get_group_info(self, group_id).await
    }
    async fn get_group_list(&self) -> Result<Vec<GroupInfo>, MilkyClientError> {
        MilkyClient::get_group_list(self).await
    }
    async fn get_group_member_info(
        &self,
        group_id: i64,
        user_id: i64,
    ) -> Result<GroupMemberInfo, MilkyClientError> {
        MilkyClient::get_group_member_info(self, group_id, user_id).await
    }
    async fn get_group_member_list(
        &self,
        group_id: i64,
    ) -> Result<Vec<GroupMemberInfo>, MilkyClientError> {
        MilkyClient::get_group_member_list(self, group_id).await
    }
    async fn get_message(
        &self,
        message_ref: &MessageRef,
    ) -> Result<InboundEvent, MilkyClientError> {
        MilkyClient::get_message(self, message_ref).await
    }
    async fn delete_message(&self, message_ref: &MessageRef) -> Result<(), MilkyClientError> {
        MilkyClient::delete_message(self, message_ref).await
    }
    async fn handle_friend_request(
        &self,
        request: &RequestRef,
        approve: bool,
        reason: String,
    ) -> Result<(), MilkyClientError> {
        MilkyClient::handle_friend_request(self, request, approve, reason).await
    }
    async fn handle_group_request(
        &self,
        request: &RequestRef,
        approve: bool,
    ) -> Result<(), MilkyClientError> {
        MilkyClient::handle_group_request(self, request, approve).await
    }
    async fn connect(&self) -> Result<LoginInfo, MilkyClientError> {
        MilkyClient::connect(self).await
    }
    async fn shutdown(&self) {
        MilkyClient::shutdown(self).await
    }
}

pub struct Service {
    cfg: Config,
    upstream: Arc<dyn Upstream>,
    runtime: Arc<Runtime>,
    messages: Arc<MessageMap>,
    requests: Arc<RequestMap>,
}

impl Service {
    pub fn new(cfg: Config, upstream: Arc<MilkyClient>) -> Arc<Self> {
        Self::with_upstream(cfg, upstream as Arc<dyn Upstream>)
    }

    fn with_upstream(cfg: Config, upstream: Arc<dyn Upstream>) -> Arc<Self> {
        let messages = Arc::new(MessageMap::new(cfg.bridge.cache_size));
        Arc::new(Self {
            cfg,
            upstream,
            runtime: Arc::new(Runtime::new()),
            messages,
            requests: Arc::new(RequestMap::new()),
        })
    }

    pub async fn connect(&self) -> Result<(), ServiceError> {
        let mut login = self.upstream.connect().await?;
        if self.cfg.bridge.self_id != 0 {
            login.self_id = self.cfg.bridge.self_id;
        }
        self.runtime.set_login(login);
        self.runtime.set_upstream_connected(true);
        Ok(())
    }

    pub async fn shutdown(&self) {
        self.runtime.set_upstream_connected(false);
        self.upstream.shutdown().await;
    }

    pub async fn run(
        self: Arc<Self>,
        server: Arc<Server>,
        mut inbound_rx: mpsc::Receiver<InboundEvent>,
        mut shutdown: watch::Receiver<bool>,
    ) {
        let mut ticker = interval(Duration::from_millis(
            self.cfg.bridge.heartbeat_interval_ms.max(1),
        ));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        // Skip the immediate first tick — Go's NewTicker only fires after the first interval.
        ticker.tick().await;

        loop {
            tokio::select! {
                _ = shutdown.changed() => break,
                event = inbound_rx.recv() => {
                    let Some(event) = event else { break };
                    if let Some(payload) = self.translate(event) {
                        server.broadcast(payload).await;
                    }
                }
                _ = ticker.tick() => {
                    let status = self.runtime.status();
                    let payload = heartbeat_event(
                        self.self_id(),
                        json!({"online": status.online, "good": status.good}),
                        self.cfg.bridge.heartbeat_interval_ms,
                    );
                    server.broadcast(payload).await;
                }
            }
        }
    }

    fn translate(&self, event: InboundEvent) -> Option<Value> {
        translate_event(
            event,
            self.self_id(),
            &self.cfg.bridge.message_format,
            &self.messages,
            &self.requests,
        )
    }

    fn self_id(&self) -> i64 {
        let login = self.runtime.login();
        if login.self_id != 0 {
            return login.self_id;
        }
        self.cfg.bridge.self_id
    }

    fn runtime_login(&self) -> LoginInfo {
        self.runtime.login()
    }
}

#[async_trait]
impl Handler for Service {
    fn current_self_id(&self) -> i64 {
        self.self_id()
    }

    async fn on_ws_connect(&self, role: &str) -> Vec<Value> {
        if role == "api" || role == "reverse-api" {
            return Vec::new();
        }
        vec![lifecycle_event(self.self_id(), "connect")]
    }

    async fn handle_api(&self, req: ApiRequest) -> ApiResponse {
        let action = normalize_action(&req.action);
        let echo = req.echo.clone();
        let params = req.params.clone();

        match action.as_str() {
            "send_private_msg" => self.action_send_private(params, echo).await,
            "send_group_msg" => self.action_send_group(params, echo).await,
            "send_msg" => self.action_send_msg(params, echo).await,
            "get_login_info" => {
                let login = self.runtime_login();
                success(
                    json!({"user_id": login.self_id, "nickname": login.nickname}),
                    echo,
                )
            }
            "get_status" => {
                let status = self.runtime.status();
                success(json!({"online": status.online, "good": status.good}), echo)
            }
            "get_version_info" => success(
                json!({
                    "app_name": "milky-ob11-bridge",
                    "app_version": "0.1.0",
                    "protocol_version": "v11",
                }),
                echo,
            ),
            "can_send_image" | "can_send_record" => success(json!({"yes": true}), echo),
            "get_group_info" => self.action_get_group_info(params, echo).await,
            "get_group_list" => self.action_get_group_list(echo).await,
            "get_group_member_info" => self.action_get_group_member_info(params, echo).await,
            "get_group_member_list" => self.action_get_group_member_list(params, echo).await,
            "delete_msg" => self.action_delete_msg(params, echo).await,
            "get_msg" => self.action_get_msg(params, echo).await,
            "set_friend_add_request" => self.action_set_friend_add_request(params, echo).await,
            "set_group_add_request" => self.action_set_group_add_request(params, echo).await,
            _ => failure(1503, unsupported_action(&action), echo),
        }
    }
}

// ---- action handlers ---------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
struct SendPrivateParams {
    #[serde(default)]
    user_id: i64,
    #[serde(default)]
    message: Value,
    #[serde(default)]
    auto_escape: bool,
}

#[derive(Debug, Default, Deserialize)]
struct SendGroupParams {
    #[serde(default)]
    group_id: i64,
    #[serde(default)]
    message: Value,
    #[serde(default)]
    auto_escape: bool,
}

#[derive(Debug, Default, Deserialize)]
struct SendMsgParams {
    #[serde(default)]
    message_type: String,
    #[serde(default)]
    user_id: i64,
    #[serde(default)]
    group_id: i64,
    #[serde(default)]
    message: Value,
    #[serde(default)]
    auto_escape: bool,
}

#[derive(Debug, Default, Deserialize)]
struct GroupIdParams {
    #[serde(default)]
    group_id: i64,
}

#[derive(Debug, Default, Deserialize)]
struct GroupMemberParams {
    #[serde(default)]
    group_id: i64,
    #[serde(default)]
    user_id: i64,
}

#[derive(Debug, Default, Deserialize)]
struct MessageIdParams {
    #[serde(default)]
    message_id: i64,
}

#[derive(Debug, Default, Deserialize)]
struct FriendRequestParams {
    #[serde(default)]
    flag: String,
    #[serde(default)]
    approve: bool,
    #[serde(default)]
    remark: String,
}

#[derive(Debug, Default, Deserialize)]
struct GroupRequestParams {
    #[serde(default)]
    flag: String,
    #[serde(default)]
    approve: bool,
    // Accepted for OneBot-11 wire compatibility, but the Milky group-invitation
    // endpoint takes no rejection reason, so we drop it.
    #[serde(default, rename = "reason")]
    _reason: String,
}

#[allow(clippy::result_large_err)]
fn decode<T: Default + for<'de> Deserialize<'de>>(
    params: Option<Value>,
    echo: &Option<Value>,
) -> Result<T, ApiResponse> {
    let value = params.unwrap_or(Value::Null);
    if value.is_null() {
        return Ok(T::default());
    }
    serde_json::from_value::<T>(value).map_err(|e| failure(1400, e.to_string(), echo.clone()))
}

impl Service {
    async fn action_send_private(&self, params: Option<Value>, echo: Option<Value>) -> ApiResponse {
        let p: SendPrivateParams = match decode(params, &echo) {
            Ok(p) => p,
            Err(r) => return r,
        };
        let segments = match parse_message(&p.message, p.auto_escape, &echo) {
            Ok(s) => s,
            Err(r) => return r,
        };
        match self
            .upstream
            .send_private_message(p.user_id, segments)
            .await
        {
            Ok(message_id) => {
                self.messages.put(MessageRef {
                    onebot_id: message_id,
                    milky_seq: message_id,
                    message_type: "private".into(),
                    user_id: p.user_id,
                    ..MessageRef::default()
                });
                success(json!({"message_id": message_id}), echo)
            }
            Err(e) => failure(1500, e.to_string(), echo),
        }
    }

    async fn action_send_group(&self, params: Option<Value>, echo: Option<Value>) -> ApiResponse {
        let p: SendGroupParams = match decode(params, &echo) {
            Ok(p) => p,
            Err(r) => return r,
        };
        let segments = match parse_message(&p.message, p.auto_escape, &echo) {
            Ok(s) => s,
            Err(r) => return r,
        };
        match self.upstream.send_group_message(p.group_id, segments).await {
            Ok(message_id) => {
                self.messages.put(MessageRef {
                    onebot_id: message_id,
                    milky_seq: message_id,
                    message_type: "group".into(),
                    group_id: p.group_id,
                    ..MessageRef::default()
                });
                success(json!({"message_id": message_id}), echo)
            }
            Err(e) => failure(1500, e.to_string(), echo),
        }
    }

    async fn action_send_msg(&self, params: Option<Value>, echo: Option<Value>) -> ApiResponse {
        let p: SendMsgParams = match decode(params, &echo) {
            Ok(p) => p,
            Err(r) => return r,
        };
        let segments = match parse_message(&p.message, p.auto_escape, &echo) {
            Ok(s) => s,
            Err(r) => return r,
        };
        match p.message_type.as_str() {
            "" | "private" if p.user_id != 0 => {
                match self
                    .upstream
                    .send_private_message(p.user_id, segments)
                    .await
                {
                    Ok(message_id) => {
                        self.messages.put(MessageRef {
                            onebot_id: message_id,
                            milky_seq: message_id,
                            message_type: "private".into(),
                            user_id: p.user_id,
                            ..MessageRef::default()
                        });
                        success(json!({"message_id": message_id}), echo)
                    }
                    Err(e) => failure(1500, e.to_string(), echo),
                }
            }
            "group" if p.group_id != 0 => {
                match self.upstream.send_group_message(p.group_id, segments).await {
                    Ok(message_id) => {
                        self.messages.put(MessageRef {
                            onebot_id: message_id,
                            milky_seq: message_id,
                            message_type: "group".into(),
                            group_id: p.group_id,
                            ..MessageRef::default()
                        });
                        success(json!({"message_id": message_id}), echo)
                    }
                    Err(e) => failure(1500, e.to_string(), echo),
                }
            }
            _ => failure(1400, "send_msg requires a valid message target", echo),
        }
    }

    async fn action_get_group_info(
        &self,
        params: Option<Value>,
        echo: Option<Value>,
    ) -> ApiResponse {
        let p: GroupIdParams = match decode(params, &echo) {
            Ok(p) => p,
            Err(r) => return r,
        };
        match self.upstream.get_group_info(p.group_id).await {
            Ok(info) => match serde_json::to_value(info) {
                Ok(v) => success(v, echo),
                Err(e) => failure(1500, e.to_string(), echo),
            },
            Err(e) => failure(1500, e.to_string(), echo),
        }
    }

    async fn action_get_group_list(&self, echo: Option<Value>) -> ApiResponse {
        match self.upstream.get_group_list().await {
            Ok(list) => match serde_json::to_value(list) {
                Ok(v) => success(v, echo),
                Err(e) => failure(1500, e.to_string(), echo),
            },
            Err(e) => failure(1500, e.to_string(), echo),
        }
    }

    async fn action_get_group_member_info(
        &self,
        params: Option<Value>,
        echo: Option<Value>,
    ) -> ApiResponse {
        let p: GroupMemberParams = match decode(params, &echo) {
            Ok(p) => p,
            Err(r) => return r,
        };
        match self
            .upstream
            .get_group_member_info(p.group_id, p.user_id)
            .await
        {
            Ok(info) => match serde_json::to_value(info) {
                Ok(v) => success(v, echo),
                Err(e) => failure(1500, e.to_string(), echo),
            },
            Err(e) => failure(1500, e.to_string(), echo),
        }
    }

    async fn action_get_group_member_list(
        &self,
        params: Option<Value>,
        echo: Option<Value>,
    ) -> ApiResponse {
        let p: GroupIdParams = match decode(params, &echo) {
            Ok(p) => p,
            Err(r) => return r,
        };
        match self.upstream.get_group_member_list(p.group_id).await {
            Ok(list) => match serde_json::to_value(list) {
                Ok(v) => success(v, echo),
                Err(e) => failure(1500, e.to_string(), echo),
            },
            Err(e) => failure(1500, e.to_string(), echo),
        }
    }

    async fn action_delete_msg(&self, params: Option<Value>, echo: Option<Value>) -> ApiResponse {
        let p: MessageIdParams = match decode(params, &echo) {
            Ok(p) => p,
            Err(r) => return r,
        };
        let Some(reff) = self.messages.get(p.message_id) else {
            return failure(1502, "message_id not found", echo);
        };
        match self.upstream.delete_message(&reff).await {
            Ok(()) => success(Value::Null, echo),
            Err(e) => failure(1500, e.to_string(), echo),
        }
    }

    async fn action_get_msg(&self, params: Option<Value>, echo: Option<Value>) -> ApiResponse {
        let p: MessageIdParams = match decode(params, &echo) {
            Ok(p) => p,
            Err(r) => return r,
        };
        let Some(reff) = self.messages.get(p.message_id) else {
            return failure(1502, "message_id not found", echo);
        };
        let event = match self.upstream.get_message(&reff).await {
            Ok(e) => e,
            Err(e) => return failure(1500, e.to_string(), echo),
        };
        let (message, raw) = super::message_ir::build_onebot_message(
            &self.cfg.bridge.message_format,
            &event.segments,
        );
        let mut data = serde_json::Map::new();
        data.insert("time".into(), Value::from(event.time));
        data.insert(
            "message_type".into(),
            Value::String(reff.message_type.clone()),
        );
        data.insert("message_id".into(), Value::from(reff.onebot_id));
        data.insert("real_id".into(), Value::from(reff.milky_seq));
        data.insert("message".into(), message);
        data.insert("raw_message".into(), Value::String(raw));
        data.insert(
            "sender".into(),
            build_get_msg_sender(&event.sender, &reff.message_type),
        );
        if reff.message_type == "group" {
            data.insert("group_id".into(), Value::from(reff.group_id));
        } else {
            data.insert("user_id".into(), Value::from(reff.user_id));
        }
        success(Value::Object(data), echo)
    }

    async fn action_set_friend_add_request(
        &self,
        params: Option<Value>,
        echo: Option<Value>,
    ) -> ApiResponse {
        let p: FriendRequestParams = match decode(params, &echo) {
            Ok(p) => p,
            Err(r) => return r,
        };
        let Some(reff) = self.requests.get(&p.flag) else {
            return failure(1502, "request flag not found", echo);
        };
        if reff.kind != "friend" {
            return failure(1400, "flag does not point to a friend request", echo);
        }
        match self
            .upstream
            .handle_friend_request(&reff, p.approve, p.remark)
            .await
        {
            Ok(()) => success(Value::Null, echo),
            Err(e) => failure(1500, e.to_string(), echo),
        }
    }

    async fn action_set_group_add_request(
        &self,
        params: Option<Value>,
        echo: Option<Value>,
    ) -> ApiResponse {
        let p: GroupRequestParams = match decode(params, &echo) {
            Ok(p) => p,
            Err(r) => return r,
        };
        let Some(reff) = self.requests.get(&p.flag) else {
            return failure(1502, "request flag not found", echo);
        };
        if reff.kind != "group" {
            return failure(1400, "flag does not point to a group request", echo);
        }
        match self.upstream.handle_group_request(&reff, p.approve).await {
            Ok(()) => success(Value::Null, echo),
            Err(e) => failure(1500, e.to_string(), echo),
        }
    }
}

#[allow(clippy::result_large_err)]
fn parse_message(
    raw: &Value,
    auto_escape: bool,
    echo: &Option<Value>,
) -> Result<Vec<Segment>, ApiResponse> {
    parse_onebot_message(raw, auto_escape).map_err(|e| failure(1400, e.to_string(), echo.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{BridgeConfig, MilkyConfig, OneBotConfig};
    use crate::types::{EventKind, SegmentType, Sender};
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn cfg() -> Config {
        Config {
            milky: MilkyConfig {
                ws_endpoint: "ws://127.0.0.1:1".into(),
                token: String::new(),
            },
            onebot: OneBotConfig::default(),
            bridge: BridgeConfig::default(),
        }
    }

    #[derive(Default)]
    struct StubUpstream {
        sends: AtomicUsize,
    }

    #[async_trait]
    impl Upstream for StubUpstream {
        async fn send_private_message(
            &self,
            _user_id: i64,
            _segments: Vec<Segment>,
        ) -> Result<i64, MilkyClientError> {
            let n = self.sends.fetch_add(1, Ordering::SeqCst) as i64;
            Ok(1000 + n)
        }
        async fn send_group_message(
            &self,
            _group_id: i64,
            _segments: Vec<Segment>,
        ) -> Result<i64, MilkyClientError> {
            let n = self.sends.fetch_add(1, Ordering::SeqCst) as i64;
            Ok(2000 + n)
        }
        async fn get_group_info(
            &self,
            group_id: i64,
        ) -> Result<crate::types::GroupInfo, MilkyClientError> {
            Ok(crate::types::GroupInfo {
                group_id,
                group_name: "g".into(),
                member_count: 10,
                max_member_count: 100,
            })
        }
        async fn get_group_list(&self) -> Result<Vec<crate::types::GroupInfo>, MilkyClientError> {
            Ok(Vec::new())
        }
        async fn get_group_member_info(
            &self,
            group_id: i64,
            user_id: i64,
        ) -> Result<crate::types::GroupMemberInfo, MilkyClientError> {
            Ok(crate::types::GroupMemberInfo {
                group_id,
                user_id,
                ..crate::types::GroupMemberInfo::default()
            })
        }
        async fn get_group_member_list(
            &self,
            _group_id: i64,
        ) -> Result<Vec<crate::types::GroupMemberInfo>, MilkyClientError> {
            Ok(Vec::new())
        }
        async fn get_message(
            &self,
            _message_ref: &MessageRef,
        ) -> Result<InboundEvent, MilkyClientError> {
            Err(MilkyClientError::NotConnected)
        }
        async fn delete_message(&self, _message_ref: &MessageRef) -> Result<(), MilkyClientError> {
            Ok(())
        }
        async fn handle_friend_request(
            &self,
            _request: &RequestRef,
            _approve: bool,
            _reason: String,
        ) -> Result<(), MilkyClientError> {
            Ok(())
        }
        async fn handle_group_request(
            &self,
            _request: &RequestRef,
            _approve: bool,
        ) -> Result<(), MilkyClientError> {
            Ok(())
        }
        async fn connect(&self) -> Result<LoginInfo, MilkyClientError> {
            Ok(LoginInfo {
                self_id: 1,
                nickname: "stub".into(),
            })
        }
        async fn shutdown(&self) {}
    }

    fn stub_service() -> Arc<Service> {
        Service::with_upstream(cfg(), Arc::new(StubUpstream::default()))
    }

    #[tokio::test]
    async fn unsupported_action_returns_1503() {
        let svc = stub_service();
        let resp = svc
            .handle_api(ApiRequest {
                action: "totally_made_up".into(),
                params: None,
                echo: Some(json!("e")),
            })
            .await;
        assert_eq!(resp.status, "failed");
        assert_eq!(resp.retcode, 1503);
        assert_eq!(resp.echo, Some(json!("e")));
    }

    #[tokio::test]
    async fn get_login_info_reads_runtime() {
        let svc = stub_service();
        svc.runtime.set_login(LoginInfo {
            self_id: 42,
            nickname: "bot".into(),
        });
        let resp = svc
            .handle_api(ApiRequest {
                action: "get_login_info".into(),
                params: None,
                echo: None,
            })
            .await;
        assert_eq!(resp.status, "ok");
        assert_eq!(resp.data["user_id"], 42);
        assert_eq!(resp.data["nickname"], "bot");
    }

    #[tokio::test]
    async fn get_status_includes_online_flag() {
        let svc = stub_service();
        svc.runtime.set_upstream_connected(true);
        svc.runtime.set_login(LoginInfo {
            self_id: 7,
            nickname: "n".into(),
        });
        let resp = svc
            .handle_api(ApiRequest {
                action: "get_status".into(),
                params: None,
                echo: None,
            })
            .await;
        assert_eq!(resp.data["online"], true);
        assert_eq!(resp.data["good"], true);
    }

    #[tokio::test]
    async fn get_version_info_returns_static() {
        let svc = stub_service();
        let resp = svc
            .handle_api(ApiRequest {
                action: "get_version_info".into(),
                params: None,
                echo: None,
            })
            .await;
        assert_eq!(resp.data["app_name"], "milky-ob11-bridge");
        assert_eq!(resp.data["protocol_version"], "v11");
    }

    #[tokio::test]
    async fn can_send_image_returns_yes() {
        let svc = stub_service();
        let resp = svc
            .handle_api(ApiRequest {
                action: "can_send_image".into(),
                params: None,
                echo: None,
            })
            .await;
        assert_eq!(resp.data["yes"], true);
    }

    #[tokio::test]
    async fn send_private_msg_caches_message_ref() {
        let svc = stub_service();
        let resp = svc
            .handle_api(ApiRequest {
                action: "send_private_msg".into(),
                params: Some(json!({
                    "user_id": 5,
                    "message": [{"type": "text", "data": {"text": "hi"}}],
                })),
                echo: None,
            })
            .await;
        assert_eq!(resp.status, "ok");
        let mid = resp.data["message_id"].as_i64().unwrap();
        let cached = svc.messages.get(mid).unwrap();
        assert_eq!(cached.message_type, "private");
        assert_eq!(cached.user_id, 5);
    }

    #[tokio::test]
    async fn send_group_msg_caches_message_ref() {
        let svc = stub_service();
        let resp = svc
            .handle_api(ApiRequest {
                action: "send_group_msg".into(),
                params: Some(json!({
                    "group_id": 9,
                    "message": "[CQ:at,qq=1] hi",
                })),
                echo: None,
            })
            .await;
        assert_eq!(resp.status, "ok");
        let mid = resp.data["message_id"].as_i64().unwrap();
        let cached = svc.messages.get(mid).unwrap();
        assert_eq!(cached.message_type, "group");
        assert_eq!(cached.group_id, 9);
    }

    #[tokio::test]
    async fn send_msg_routes_by_type() {
        let svc = stub_service();
        let resp = svc
            .handle_api(ApiRequest {
                action: "send_msg".into(),
                params: Some(json!({
                    "message_type": "group",
                    "group_id": 100,
                    "message": [{"type": "text", "data": {"text": "x"}}],
                })),
                echo: None,
            })
            .await;
        assert_eq!(resp.status, "ok");
    }

    #[tokio::test]
    async fn send_msg_missing_target_is_1400() {
        let svc = stub_service();
        let resp = svc
            .handle_api(ApiRequest {
                action: "send_msg".into(),
                params: Some(json!({
                    "message_type": "private",
                    "message": [{"type": "text", "data": {"text": "x"}}],
                })),
                echo: None,
            })
            .await;
        assert_eq!(resp.retcode, 1400);
    }

    #[tokio::test]
    async fn set_friend_request_unknown_flag_is_1502() {
        let svc = stub_service();
        let resp = svc
            .handle_api(ApiRequest {
                action: "set_friend_add_request".into(),
                params: Some(json!({"flag": "ghost", "approve": true})),
                echo: None,
            })
            .await;
        assert_eq!(resp.retcode, 1502);
    }

    #[tokio::test]
    async fn set_group_request_kind_mismatch_is_1400() {
        let svc = stub_service();
        let flag = svc.requests.put(RequestRef {
            kind: "friend".into(),
            initiator_uid: 1,
            ..RequestRef::default()
        });
        let resp = svc
            .handle_api(ApiRequest {
                action: "set_group_add_request".into(),
                params: Some(json!({"flag": flag, "approve": true})),
                echo: None,
            })
            .await;
        assert_eq!(resp.retcode, 1400);
        assert!(resp.msg.contains("group request"));
    }

    #[tokio::test]
    async fn set_friend_request_ok_path_passes_remark() {
        let svc = stub_service();
        let flag = svc.requests.put(RequestRef {
            kind: "friend".into(),
            initiator_uid: 99,
            ..RequestRef::default()
        });
        let resp = svc
            .handle_api(ApiRequest {
                action: "set_friend_add_request".into(),
                params: Some(json!({"flag": flag, "approve": false, "remark": "no"})),
                echo: None,
            })
            .await;
        assert_eq!(resp.status, "ok");
    }

    #[tokio::test]
    async fn delete_msg_missing_id_is_1502() {
        let svc = stub_service();
        let resp = svc
            .handle_api(ApiRequest {
                action: "delete_msg".into(),
                params: Some(json!({"message_id": 99})),
                echo: None,
            })
            .await;
        assert_eq!(resp.retcode, 1502);
    }

    #[tokio::test]
    async fn delete_msg_known_id_succeeds() {
        let svc = stub_service();
        svc.messages.put(MessageRef {
            onebot_id: 7,
            milky_seq: 7,
            message_type: "private".into(),
            user_id: 11,
            ..MessageRef::default()
        });
        let resp = svc
            .handle_api(ApiRequest {
                action: "delete_msg".into(),
                params: Some(json!({"message_id": 7})),
                echo: None,
            })
            .await;
        assert_eq!(resp.status, "ok");
    }

    #[tokio::test]
    async fn get_group_info_passes_through() {
        let svc = stub_service();
        let resp = svc
            .handle_api(ApiRequest {
                action: "get_group_info".into(),
                params: Some(json!({"group_id": 42})),
                echo: None,
            })
            .await;
        assert_eq!(resp.status, "ok");
        assert_eq!(resp.data["group_id"], 42);
    }

    #[tokio::test]
    async fn on_ws_connect_skips_api_role() {
        let svc = stub_service();
        assert!(svc.on_ws_connect("api").await.is_empty());
        assert!(svc.on_ws_connect("reverse-api").await.is_empty());
        let evs = svc.on_ws_connect("universal").await;
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0]["meta_event_type"], "lifecycle");
    }

    #[tokio::test]
    async fn current_self_id_falls_back_to_cfg() {
        let mut c = cfg();
        c.bridge.self_id = 555;
        let svc = Service::with_upstream(c, Arc::new(StubUpstream::default()));
        assert_eq!(svc.current_self_id(), 555);
        svc.runtime.set_login(LoginInfo {
            self_id: 999,
            nickname: "n".into(),
        });
        assert_eq!(svc.current_self_id(), 999);
    }

    #[tokio::test]
    async fn translate_caches_inbound_private_message() {
        let svc = stub_service();
        let mut data = BTreeMap::new();
        data.insert("text".into(), "hi".into());
        let event = InboundEvent {
            kind: EventKind::MessagePrivate,
            time: 1,
            message_id: 7,
            group_id: 0,
            user_id: 100,
            target_id: 0,
            segments: vec![Segment {
                kind: SegmentType::Text,
                data,
                raw: BTreeMap::new(),
            }],
            sender: Sender::default(),
            comment: String::new(),
            request: None,
        };
        let payload = svc.translate(event).unwrap();
        assert_eq!(payload["post_type"], "message");
        assert!(svc.messages.get(7).is_some());
    }

    #[tokio::test]
    async fn connect_promotes_runtime_to_online() {
        let svc = stub_service();
        svc.connect().await.unwrap();
        assert!(svc.runtime.status().online);
        assert_eq!(svc.runtime.login().nickname, "stub");
    }

    #[tokio::test]
    async fn connect_overrides_self_id_from_cfg() {
        let mut c = cfg();
        c.bridge.self_id = 4242;
        let svc = Service::with_upstream(c, Arc::new(StubUpstream::default()));
        svc.connect().await.unwrap();
        assert_eq!(svc.runtime.login().self_id, 4242);
    }
}
