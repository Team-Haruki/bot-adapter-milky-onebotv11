use std::sync::Arc;

use milky_rust_sdk::prelude::{Event as SdkEvent, MessageScene};
use milky_rust_sdk::{Communication, MilkyClient, WebSocketConfig};
use tokio::sync::{Mutex, mpsc};

use super::error::MilkyClientError;
use super::events;
use super::segments;
use crate::config::MilkyConfig;
use crate::types::{
    EventKind, GroupInfo, GroupMemberInfo, InboundEvent, LoginInfo, MessageRef, RequestRef, Segment,
};

const SDK_EVENT_BUFFER: usize = 128;

pub struct Client {
    sdk: Arc<MilkyClient>,
    inbound_tx: mpsc::Sender<InboundEvent>,
    sdk_event_rx: Mutex<Option<mpsc::Receiver<SdkEvent>>>,
}

impl Client {
    pub fn new(
        cfg: &MilkyConfig,
        inbound_tx: mpsc::Sender<InboundEvent>,
    ) -> Result<Self, MilkyClientError> {
        let token = if cfg.token.is_empty() {
            None
        } else {
            Some(cfg.token.clone())
        };
        let ws = WebSocketConfig::new(cfg.ws_endpoint.clone(), token);
        let (sdk_tx, sdk_rx) = mpsc::channel::<SdkEvent>(SDK_EVENT_BUFFER);
        let sdk = MilkyClient::new(Communication::WebSocket(ws), sdk_tx)?;
        Ok(Self {
            sdk: Arc::new(sdk),
            inbound_tx,
            sdk_event_rx: Mutex::new(Some(sdk_rx)),
        })
    }

    pub async fn connect(&self) -> Result<LoginInfo, MilkyClientError> {
        self.sdk.connect_events().await?;
        let info = self.sdk.get_login_info().await?;
        let login = LoginInfo {
            self_id: info.uin,
            nickname: info.nickname,
        };

        let mut rx = self
            .sdk_event_rx
            .lock()
            .await
            .take()
            .ok_or(MilkyClientError::NotConnected)?;
        let inbound_tx = self.inbound_tx.clone();
        let self_id = login.self_id;
        tokio::spawn(async move {
            while let Some(ev) = rx.recv().await {
                if let Some(out) = events::translate_event(ev, self_id)
                    && inbound_tx.send(out).await.is_err()
                {
                    tracing::warn!("inbound channel closed, stopping translator");
                    break;
                }
            }
        });
        Ok(login)
    }

    pub async fn shutdown(&self) {
        self.sdk.shutdown().await;
    }

    pub async fn send_private_message(
        &self,
        user_id: i64,
        segments: Vec<Segment>,
    ) -> Result<i64, MilkyClientError> {
        let outgoing = segments::to_outgoing(segments)?;
        let resp = self.sdk.send_private_message(user_id, outgoing).await?;
        Ok(resp.message_seq)
    }

    pub async fn send_group_message(
        &self,
        group_id: i64,
        segments: Vec<Segment>,
    ) -> Result<i64, MilkyClientError> {
        let outgoing = segments::to_outgoing(segments)?;
        let resp = self.sdk.send_group_message(group_id, outgoing).await?;
        Ok(resp.message_seq)
    }

    pub async fn get_group_info(&self, group_id: i64) -> Result<GroupInfo, MilkyClientError> {
        let resp = self.sdk.get_group_info(group_id, true).await?;
        Ok(GroupInfo {
            group_id: resp.group.group_id,
            group_name: resp.group.group_name,
            member_count: resp.group.member_count,
            max_member_count: resp.group.max_member_count,
        })
    }

    pub async fn get_group_list(&self) -> Result<Vec<GroupInfo>, MilkyClientError> {
        let resp = self.sdk.get_group_list(true).await?;
        Ok(resp
            .groups
            .into_iter()
            .map(|g| GroupInfo {
                group_id: g.group_id,
                group_name: g.group_name,
                member_count: g.member_count,
                max_member_count: g.max_member_count,
            })
            .collect())
    }

    pub async fn get_group_member_info(
        &self,
        group_id: i64,
        user_id: i64,
    ) -> Result<GroupMemberInfo, MilkyClientError> {
        let resp = self
            .sdk
            .get_group_member_info(group_id, user_id, true)
            .await?;
        Ok(group_member_to_info(resp.member))
    }

    pub async fn get_group_member_list(
        &self,
        group_id: i64,
    ) -> Result<Vec<GroupMemberInfo>, MilkyClientError> {
        let resp = self.sdk.get_group_member_list(group_id, true).await?;
        Ok(resp.members.into_iter().map(group_member_to_info).collect())
    }

    pub async fn get_message(
        &self,
        message_ref: &MessageRef,
    ) -> Result<InboundEvent, MilkyClientError> {
        let (scene, peer_id, kind) = match message_ref.message_type.as_str() {
            "group" => (
                MessageScene::Group,
                message_ref.group_id,
                EventKind::MessageGroup,
            ),
            _ => (
                MessageScene::Friend,
                message_ref.user_id,
                EventKind::MessagePrivate,
            ),
        };
        let resp = self
            .sdk
            .get_message(scene, peer_id, message_ref.milky_seq)
            .await?;
        Ok(events::from_incoming_message(resp.message, kind))
    }

    pub async fn delete_message(&self, message_ref: &MessageRef) -> Result<(), MilkyClientError> {
        match message_ref.message_type.as_str() {
            "group" => {
                self.sdk
                    .recall_group_message(message_ref.group_id, message_ref.milky_seq)
                    .await?;
            }
            _ => {
                self.sdk
                    .recall_private_message(message_ref.user_id, message_ref.milky_seq)
                    .await?;
            }
        }
        Ok(())
    }

    pub async fn handle_friend_request(
        &self,
        request: &RequestRef,
        approve: bool,
        reason: String,
    ) -> Result<(), MilkyClientError> {
        if approve {
            self.sdk
                .accept_friend_request(request.initiator_uid, false)
                .await?;
        } else {
            self.sdk
                .reject_friend_request(request.initiator_uid, false, reason)
                .await?;
        }
        Ok(())
    }

    pub async fn handle_group_request(
        &self,
        request: &RequestRef,
        approve: bool,
    ) -> Result<(), MilkyClientError> {
        let seq = request.invitation_seq.to_string();
        if approve {
            self.sdk
                .accept_group_invitation(request.group_id, seq)
                .await?;
        } else {
            self.sdk
                .reject_group_invitation(request.group_id, seq)
                .await?;
        }
        Ok(())
    }
}

fn group_member_to_info(m: milky_rust_sdk::prelude::GroupMember) -> GroupMemberInfo {
    GroupMemberInfo {
        group_id: m.group_id,
        user_id: m.user_id,
        nickname: m.nickname,
        card: m.card,
        sex: format!("{:?}", m.sex).to_lowercase(),
        age: 0,
        area: String::new(),
        level: m.level.to_string(),
        role: format!("{:?}", m.role).to_lowercase(),
        title: m.title,
    }
}
