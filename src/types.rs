use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SegmentType {
    Text,
    Image,
    At,
    Reply,
    Record,
    File,
    Poke,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    #[serde(rename = "type")]
    pub kind: SegmentType,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub data: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub raw: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    MessagePrivate,
    MessageGroup,
    PokePrivate,
    PokeGroup,
    FriendRequest,
    GroupInvite,
    RecallPrivate,
    RecallGroup,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Sender {
    pub user_id: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub nickname: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub card: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sex: String,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub age: i32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub area: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub level: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub role: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RequestRef {
    pub kind: String,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub initiator_uid: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub group_id: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub invitation_seq: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundEvent {
    pub kind: EventKind,
    pub time: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub message_id: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub group_id: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub user_id: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub target_id: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub segments: Vec<Segment>,
    #[serde(default, skip_serializing_if = "Sender::is_empty")]
    pub sender: Sender,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comment: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<RequestRef>,
}

impl Sender {
    fn is_empty(&self) -> bool {
        self.user_id == 0
            && self.nickname.is_empty()
            && self.card.is_empty()
            && self.sex.is_empty()
            && self.age == 0
            && self.area.is_empty()
            && self.level.is_empty()
            && self.role.is_empty()
            && self.title.is_empty()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LoginInfo {
    pub self_id: i64,
    pub nickname: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GroupInfo {
    pub group_id: i64,
    pub group_name: String,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub member_count: i32,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub max_member_count: i32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GroupMemberInfo {
    pub group_id: i64,
    pub user_id: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub nickname: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub card: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sex: String,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub age: i32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub area: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub level: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub role: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Status {
    pub online: bool,
    pub good: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageRef {
    pub onebot_id: i64,
    pub milky_seq: i64,
    pub message_type: String,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub group_id: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub user_id: i64,
}

fn is_zero_i32(v: &i32) -> bool {
    *v == 0
}

fn is_zero_i64(v: &i64) -> bool {
    *v == 0
}
