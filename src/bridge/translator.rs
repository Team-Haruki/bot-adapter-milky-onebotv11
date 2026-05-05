use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use super::message_ir::build_onebot_message;
use crate::state::{MessageMap, RequestMap};
use crate::types::{EventKind, InboundEvent, MessageRef, Segment, SegmentType, Sender};

pub fn translate_event(
    event: InboundEvent,
    self_id: i64,
    message_format: &str,
    messages: &MessageMap,
    requests: &RequestMap,
) -> Option<Value> {
    let time = choose_event_time(event.time);
    match event.kind {
        EventKind::MessagePrivate => {
            messages.put(MessageRef {
                onebot_id: event.message_id,
                milky_seq: event.message_id,
                message_type: "private".into(),
                user_id: event.user_id,
                ..MessageRef::default()
            });
            register_reply_refs(messages, &event.segments, "private", 0, event.user_id);
            let (message, raw) = build_onebot_message(message_format, &event.segments);
            Some(json!({
                "time": time,
                "self_id": self_id,
                "post_type": "message",
                "message_type": "private",
                "sub_type": "friend",
                "message_id": event.message_id,
                "user_id": event.user_id,
                "message": message,
                "raw_message": raw,
                "font": 0,
                "sender": private_sender(&event.sender),
            }))
        }
        EventKind::MessageGroup => {
            messages.put(MessageRef {
                onebot_id: event.message_id,
                milky_seq: event.message_id,
                message_type: "group".into(),
                group_id: event.group_id,
                user_id: event.user_id,
            });
            register_reply_refs(messages, &event.segments, "group", event.group_id, 0);
            let (message, raw) = build_onebot_message(message_format, &event.segments);
            Some(json!({
                "time": time,
                "self_id": self_id,
                "post_type": "message",
                "message_type": "group",
                "sub_type": "normal",
                "message_id": event.message_id,
                "group_id": event.group_id,
                "user_id": event.user_id,
                "message": message,
                "raw_message": raw,
                "font": 0,
                "sender": group_sender(&event.sender),
            }))
        }
        EventKind::PokePrivate => Some(json!({
            "time": time,
            "self_id": self_id,
            "post_type": "notice",
            "notice_type": "notify",
            "sub_type": "poke",
            "user_id": event.user_id,
            "target_id": event.target_id,
        })),
        EventKind::PokeGroup => Some(json!({
            "time": time,
            "self_id": self_id,
            "post_type": "notice",
            "notice_type": "notify",
            "sub_type": "poke",
            "group_id": event.group_id,
            "user_id": event.user_id,
            "target_id": event.target_id,
        })),
        EventKind::FriendRequest => {
            let request = event.request.clone()?;
            let flag = requests.put(request);
            Some(json!({
                "time": time,
                "self_id": self_id,
                "post_type": "request",
                "request_type": "friend",
                "user_id": event.user_id,
                "comment": event.comment,
                "flag": flag,
            }))
        }
        EventKind::GroupInvite => {
            let request = event.request.clone()?;
            let flag = requests.put(request);
            Some(json!({
                "time": time,
                "self_id": self_id,
                "post_type": "request",
                "request_type": "group",
                "sub_type": "invite",
                "group_id": event.group_id,
                "user_id": event.user_id,
                "comment": event.comment,
                "flag": flag,
            }))
        }
        EventKind::RecallPrivate => Some(json!({
            "time": time,
            "self_id": self_id,
            "post_type": "notice",
            "notice_type": "friend_recall",
            "user_id": event.user_id,
            "message_id": event.message_id,
        })),
        EventKind::RecallGroup => Some(json!({
            "time": time,
            "self_id": self_id,
            "post_type": "notice",
            "notice_type": "group_recall",
            "group_id": event.group_id,
            "user_id": event.user_id,
            "operator_id": event.user_id,
            "message_id": event.message_id,
        })),
    }
}

pub fn private_sender(sender: &Sender) -> Value {
    json!({
        "user_id": sender.user_id,
        "nickname": sender.nickname,
        "sex": empty_to_unknown(&sender.sex),
        "age": sender.age,
    })
}

pub fn group_sender(sender: &Sender) -> Value {
    json!({
        "user_id": sender.user_id,
        "nickname": sender.nickname,
        "card": sender.card,
        "sex": empty_to_unknown(&sender.sex),
        "age": sender.age,
        "area": sender.area,
        "level": sender.level,
        "role": empty_to_member(&sender.role),
        "title": sender.title,
    })
}

pub fn build_get_msg_sender(sender: &Sender, message_type: &str) -> Value {
    if message_type == "group" {
        json!({
            "user_id": sender.user_id,
            "nickname": sender.nickname,
            "card": sender.card,
            "sex": sender.sex,
            "age": sender.age,
            "area": sender.area,
            "level": sender.level,
            "role": sender.role,
            "title": sender.title,
        })
    } else {
        json!({
            "user_id": sender.user_id,
            "nickname": sender.nickname,
            "sex": sender.sex,
            "age": sender.age,
        })
    }
}

pub fn unsupported_action(action: &str) -> String {
    format!("action {action} is not supported in v1")
}

fn register_reply_refs(
    messages: &MessageMap,
    segments: &[Segment],
    message_type: &str,
    group_id: i64,
    user_id: i64,
) {
    for seg in segments {
        if seg.kind != SegmentType::Reply {
            continue;
        }
        let Some(id_str) = seg.data.get("id") else {
            continue;
        };
        let Ok(seq) = id_str.parse::<i64>() else {
            continue;
        };
        if messages.get(seq).is_some() {
            continue;
        }
        messages.put(MessageRef {
            onebot_id: seq,
            milky_seq: seq,
            message_type: message_type.into(),
            group_id,
            user_id,
        });
    }
}

fn choose_event_time(ts: i64) -> i64 {
    if ts > 0 {
        return ts;
    }
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn empty_to_unknown(v: &str) -> &str {
    if v.is_empty() { "unknown" } else { v }
}

fn empty_to_member(v: &str) -> &str {
    if v.is_empty() { "member" } else { v }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{RequestRef, Segment, SegmentType};
    use std::collections::BTreeMap;

    fn map_pair() -> (MessageMap, RequestMap) {
        (MessageMap::new(8), RequestMap::new())
    }

    fn reply_seg(seq: i64) -> Segment {
        let mut data = BTreeMap::new();
        data.insert("id".into(), seq.to_string());
        Segment {
            kind: SegmentType::Reply,
            data,
            raw: BTreeMap::new(),
        }
    }

    fn text_seg(t: &str) -> Segment {
        let mut data = BTreeMap::new();
        data.insert("text".into(), t.into());
        Segment {
            kind: SegmentType::Text,
            data,
            raw: BTreeMap::new(),
        }
    }

    #[test]
    fn private_message_translates_and_caches_ref() {
        let (msgs, reqs) = map_pair();
        let event = InboundEvent {
            kind: EventKind::MessagePrivate,
            time: 1700000000,
            message_id: 42,
            group_id: 0,
            user_id: 555,
            target_id: 0,
            segments: vec![text_seg("hi")],
            sender: Sender {
                user_id: 555,
                nickname: "alice".into(),
                ..Sender::default()
            },
            comment: String::new(),
            request: None,
        };
        let v = translate_event(event, 999, "array", &msgs, &reqs).unwrap();
        assert_eq!(v["post_type"], "message");
        assert_eq!(v["message_type"], "private");
        assert_eq!(v["sub_type"], "friend");
        assert_eq!(v["self_id"], 999);
        assert_eq!(v["message_id"], 42);
        assert_eq!(v["user_id"], 555);
        assert_eq!(v["sender"]["nickname"], "alice");
        assert_eq!(v["sender"]["sex"], "unknown");
        assert_eq!(v["message"][0]["type"], "text");
        let cached = msgs.get(42).unwrap();
        assert_eq!(cached.message_type, "private");
        assert_eq!(cached.user_id, 555);
    }

    #[test]
    fn group_reply_pre_registers_referenced_seq() {
        let (msgs, reqs) = map_pair();
        let event = InboundEvent {
            kind: EventKind::MessageGroup,
            time: 0,
            message_id: 99308,
            group_id: 12345,
            user_id: 200,
            target_id: 0,
            segments: vec![reply_seg(99304), text_seg("re")],
            sender: Sender::default(),
            comment: String::new(),
            request: None,
        };
        translate_event(event, 1, "array", &msgs, &reqs).unwrap();
        let referenced = msgs.get(99304).expect("reply target should be cached");
        assert_eq!(referenced.message_type, "group");
        assert_eq!(referenced.group_id, 12345);
        assert_eq!(referenced.milky_seq, 99304);
    }

    #[test]
    fn private_reply_pre_registers_referenced_seq() {
        let (msgs, reqs) = map_pair();
        let event = InboundEvent {
            kind: EventKind::MessagePrivate,
            time: 0,
            message_id: 200,
            group_id: 0,
            user_id: 555,
            target_id: 0,
            segments: vec![reply_seg(180)],
            sender: Sender::default(),
            comment: String::new(),
            request: None,
        };
        translate_event(event, 1, "array", &msgs, &reqs).unwrap();
        let referenced = msgs.get(180).expect("reply target should be cached");
        assert_eq!(referenced.message_type, "private");
        assert_eq!(referenced.user_id, 555);
    }

    #[test]
    fn group_message_uses_group_sender_defaults() {
        let (msgs, reqs) = map_pair();
        let event = InboundEvent {
            kind: EventKind::MessageGroup,
            time: 0,
            message_id: 7,
            group_id: 100,
            user_id: 200,
            target_id: 0,
            segments: vec![text_seg("hello")],
            sender: Sender {
                user_id: 200,
                nickname: "bob".into(),
                ..Sender::default()
            },
            comment: String::new(),
            request: None,
        };
        let v = translate_event(event, 1, "string", &msgs, &reqs).unwrap();
        assert_eq!(v["message_type"], "group");
        assert_eq!(v["sub_type"], "normal");
        assert_eq!(v["sender"]["role"], "member");
        assert!(v["time"].as_i64().unwrap() > 0);
        assert_eq!(v["message"], v["raw_message"]);
    }

    #[test]
    fn friend_request_registers_flag() {
        let (msgs, reqs) = map_pair();
        let event = InboundEvent {
            kind: EventKind::FriendRequest,
            time: 0,
            message_id: 0,
            group_id: 0,
            user_id: 1234,
            target_id: 0,
            segments: Vec::new(),
            sender: Sender::default(),
            comment: "let me in".into(),
            request: Some(RequestRef {
                kind: "friend".into(),
                initiator_uid: 5555,
                ..RequestRef::default()
            }),
        };
        let v = translate_event(event, 1, "array", &msgs, &reqs).unwrap();
        assert_eq!(v["request_type"], "friend");
        assert_eq!(v["comment"], "let me in");
        let flag = v["flag"].as_str().unwrap();
        let stored = reqs.get(flag).unwrap();
        assert_eq!(stored.initiator_uid, 5555);
    }

    #[test]
    fn group_invite_registers_flag_with_group_id() {
        let (msgs, reqs) = map_pair();
        let event = InboundEvent {
            kind: EventKind::GroupInvite,
            time: 0,
            message_id: 0,
            group_id: 9999,
            user_id: 11,
            target_id: 0,
            segments: Vec::new(),
            sender: Sender::default(),
            comment: String::new(),
            request: Some(RequestRef {
                kind: "group".into(),
                group_id: 9999,
                invitation_seq: 7,
                ..RequestRef::default()
            }),
        };
        let v = translate_event(event, 1, "array", &msgs, &reqs).unwrap();
        assert_eq!(v["request_type"], "group");
        assert_eq!(v["sub_type"], "invite");
        assert_eq!(v["group_id"], 9999);
        let flag = v["flag"].as_str().unwrap();
        let stored = reqs.get(flag).unwrap();
        assert_eq!(stored.group_id, 9999);
        assert_eq!(stored.invitation_seq, 7);
    }

    #[test]
    fn poke_private_includes_target() {
        let (msgs, reqs) = map_pair();
        let event = InboundEvent {
            kind: EventKind::PokePrivate,
            time: 0,
            message_id: 0,
            group_id: 0,
            user_id: 1,
            target_id: 2,
            segments: Vec::new(),
            sender: Sender::default(),
            comment: String::new(),
            request: None,
        };
        let v = translate_event(event, 9, "array", &msgs, &reqs).unwrap();
        assert_eq!(v["notice_type"], "notify");
        assert_eq!(v["sub_type"], "poke");
        assert_eq!(v["user_id"], 1);
        assert_eq!(v["target_id"], 2);
    }

    #[test]
    fn group_recall_sets_operator_to_user() {
        let (msgs, reqs) = map_pair();
        let event = InboundEvent {
            kind: EventKind::RecallGroup,
            time: 0,
            message_id: 88,
            group_id: 100,
            user_id: 200,
            target_id: 0,
            segments: Vec::new(),
            sender: Sender::default(),
            comment: String::new(),
            request: None,
        };
        let v = translate_event(event, 9, "array", &msgs, &reqs).unwrap();
        assert_eq!(v["notice_type"], "group_recall");
        assert_eq!(v["operator_id"], 200);
        assert_eq!(v["user_id"], 200);
        assert_eq!(v["message_id"], 88);
    }

    #[test]
    fn unsupported_action_text_matches_go() {
        assert_eq!(
            unsupported_action("foo"),
            "action foo is not supported in v1"
        );
    }
}
