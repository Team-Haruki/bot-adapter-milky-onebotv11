use milky_rust_sdk::prelude::{
    Event, EventKind, FriendMessage, GroupMessage, IncomingMessage, MessageEvent, MessageScene,
};

use super::segments::from_incoming;
use crate::types::{EventKind as OutboundKind, InboundEvent, RequestRef, Segment, Sender};

/// Translate an SDK event into the bridge's normalized inbound event, or
/// `None` for events we ignore (Temp messages, BotOffline, group admin
/// changes, file uploads, mute changes, etc — same set Go ignores).
pub fn translate_event(ev: Event, self_id: i64) -> Option<InboundEvent> {
    match ev.kind {
        EventKind::MessageReceive { message } => match message {
            MessageEvent::Friend(m) => Some(from_friend_message(m)),
            MessageEvent::Group(m) => Some(from_group_message(m)),
            MessageEvent::Temp(_) => None,
        },
        EventKind::FriendRequest {
            initiator_id,
            initiator_uid,
            comment,
            ..
        } => Some(InboundEvent {
            kind: OutboundKind::FriendRequest,
            time: 0,
            user_id: initiator_id.parse::<i64>().unwrap_or(0),
            comment,
            request: Some(RequestRef {
                kind: "friend".into(),
                initiator_uid,
                ..RequestRef::default()
            }),
            ..empty_event()
        }),
        EventKind::GroupInvitation {
            group_id,
            invitation_seq,
            initiator_id,
        } => Some(InboundEvent {
            kind: OutboundKind::GroupInvite,
            time: 0,
            group_id,
            user_id: initiator_id,
            request: Some(RequestRef {
                kind: "group".into(),
                group_id,
                invitation_seq,
                ..RequestRef::default()
            }),
            ..empty_event()
        }),
        EventKind::FriendNudge {
            user_id,
            is_self_receive,
            ..
        } => Some(InboundEvent {
            kind: OutboundKind::PokePrivate,
            time: 0,
            user_id,
            target_id: if is_self_receive { user_id } else { self_id },
            ..empty_event()
        }),
        EventKind::GroupNudge {
            group_id,
            sender_id,
            receiver_id,
            ..
        } => Some(InboundEvent {
            kind: OutboundKind::PokeGroup,
            time: 0,
            group_id,
            user_id: sender_id,
            target_id: receiver_id,
            ..empty_event()
        }),
        EventKind::MessageRecall {
            message_scene,
            peer_id,
            message_seq,
            sender_id,
            ..
        } => match message_scene {
            MessageScene::Group => Some(InboundEvent {
                kind: OutboundKind::RecallGroup,
                time: 0,
                message_id: message_seq,
                group_id: peer_id,
                user_id: sender_id,
                ..empty_event()
            }),
            MessageScene::Friend => Some(InboundEvent {
                kind: OutboundKind::RecallPrivate,
                time: 0,
                message_id: message_seq,
                user_id: sender_id,
                ..empty_event()
            }),
            MessageScene::Temp => None,
        },
        _ => None,
    }
}

pub(super) fn from_friend_message(m: FriendMessage) -> InboundEvent {
    InboundEvent {
        kind: OutboundKind::MessagePrivate,
        time: m.message.time,
        message_id: m.message.message_seq,
        user_id: m.message.sender_id,
        segments: from_incoming(m.message.segments),
        sender: Sender {
            user_id: m.message.sender_id,
            nickname: m.friend.nickname,
            sex: format!("{:?}", m.friend.sex).to_lowercase(),
            ..Sender::default()
        },
        ..empty_event()
    }
}

pub(super) fn from_group_message(m: GroupMessage) -> InboundEvent {
    let group_id = if m.group.group_id != 0 {
        m.group.group_id
    } else {
        m.message.peer_id
    };
    InboundEvent {
        kind: OutboundKind::MessageGroup,
        time: m.message.time,
        message_id: m.message.message_seq,
        group_id,
        user_id: m.message.sender_id,
        segments: from_incoming(m.message.segments),
        sender: Sender {
            user_id: m.group_member.user_id,
            nickname: m.group_member.nickname,
            card: m.group_member.card,
            sex: format!("{:?}", m.group_member.sex).to_lowercase(),
            level: m.group_member.level.to_string(),
            role: format!("{:?}", m.group_member.role).to_lowercase(),
            title: m.group_member.title,
            ..Sender::default()
        },
        ..empty_event()
    }
}

pub(super) fn from_incoming_message(
    msg: IncomingMessage,
    fallback_kind: OutboundKind,
) -> InboundEvent {
    let group_id = if matches!(fallback_kind, OutboundKind::MessageGroup) {
        msg.peer_id
    } else {
        0
    };
    InboundEvent {
        kind: fallback_kind,
        time: msg.time,
        message_id: msg.message_seq,
        group_id,
        user_id: msg.sender_id,
        segments: from_incoming(msg.segments),
        ..empty_event()
    }
}

fn empty_event() -> InboundEvent {
    InboundEvent {
        kind: OutboundKind::MessagePrivate,
        time: 0,
        message_id: 0,
        group_id: 0,
        user_id: 0,
        target_id: 0,
        segments: Vec::<Segment>::new(),
        sender: Sender::default(),
        comment: String::new(),
        request: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use milky_rust_sdk::prelude::{
        Friend, Group, GroupMember, GroupRole, IncomingMessage, IncomingSegment, Sex,
    };

    fn friend(scene: MessageScene, sender_id: i64) -> FriendMessage {
        FriendMessage {
            message: IncomingMessage {
                peer_id: sender_id,
                message_seq: 100,
                sender_id,
                time: 1700000000,
                segments: vec![IncomingSegment::Text { text: "hi".into() }],
                message_scene: scene,
            },
            friend: Friend {
                user_id: sender_id,
                nickname: "alice".into(),
                sex: Sex::Female,
                qid: "".into(),
                remark: "".into(),
                category: None,
            },
        }
    }

    fn group_msg(group_id: i64, sender_id: i64) -> GroupMessage {
        GroupMessage {
            message: IncomingMessage {
                peer_id: group_id,
                message_seq: 200,
                sender_id,
                time: 1700000001,
                segments: vec![],
                message_scene: MessageScene::Group,
            },
            group: Group {
                group_id,
                group_name: "g".into(),
                member_count: 3,
                max_member_count: 100,
            },
            group_member: GroupMember {
                user_id: sender_id,
                nickname: "bob".into(),
                sex: Sex::Male,
                group_id,
                card: "bobby".into(),
                title: "vip".into(),
                level: 7,
                role: GroupRole::Admin,
                join_time: 0,
                last_sent_time: 0,
                shut_up_end_time: None,
            },
        }
    }

    fn ev(kind: EventKind) -> Event {
        Event {
            time: 0,
            self_id: 99,
            kind,
        }
    }

    #[test]
    fn friend_message_becomes_private() {
        let e = translate_event(
            ev(EventKind::MessageReceive {
                message: MessageEvent::Friend(friend(MessageScene::Friend, 42)),
            }),
            99,
        )
        .unwrap();
        assert_eq!(e.kind, OutboundKind::MessagePrivate);
        assert_eq!(e.user_id, 42);
        assert_eq!(e.message_id, 100);
        assert_eq!(e.sender.nickname, "alice");
        assert_eq!(e.sender.sex, "female");
        assert_eq!(e.segments.len(), 1);
    }

    #[test]
    fn group_message_populates_group_and_member() {
        let e = translate_event(
            ev(EventKind::MessageReceive {
                message: MessageEvent::Group(group_msg(555, 77)),
            }),
            99,
        )
        .unwrap();
        assert_eq!(e.kind, OutboundKind::MessageGroup);
        assert_eq!(e.group_id, 555);
        assert_eq!(e.user_id, 77);
        assert_eq!(e.sender.card, "bobby");
        assert_eq!(e.sender.level, "7");
        assert_eq!(e.sender.role, "admin");
    }

    #[test]
    fn temp_message_is_dropped() {
        let temp = milky_rust_sdk::prelude::TempMessage {
            message: IncomingMessage {
                peer_id: 1,
                message_seq: 1,
                sender_id: 1,
                time: 0,
                segments: vec![],
                message_scene: MessageScene::Temp,
            },
            group: None,
        };
        assert!(
            translate_event(
                ev(EventKind::MessageReceive {
                    message: MessageEvent::Temp(temp),
                }),
                99,
            )
            .is_none()
        );
    }

    #[test]
    fn friend_request_parses_initiator_id() {
        let e = translate_event(
            ev(EventKind::FriendRequest {
                initiator_id: "12345".into(),
                initiator_uid: 67890,
                comment: "hi".into(),
                via: "search".into(),
            }),
            99,
        )
        .unwrap();
        assert_eq!(e.kind, OutboundKind::FriendRequest);
        assert_eq!(e.user_id, 12345);
        assert_eq!(e.comment, "hi");
        let r = e.request.unwrap();
        assert_eq!(r.kind, "friend");
        assert_eq!(r.initiator_uid, 67890);
    }

    #[test]
    fn group_invitation_carries_seq() {
        let e = translate_event(
            ev(EventKind::GroupInvitation {
                group_id: 222,
                invitation_seq: 9,
                initiator_id: 33,
            }),
            99,
        )
        .unwrap();
        assert_eq!(e.kind, OutboundKind::GroupInvite);
        assert_eq!(e.group_id, 222);
        assert_eq!(e.user_id, 33);
        let r = e.request.unwrap();
        assert_eq!(r.invitation_seq, 9);
        assert_eq!(r.group_id, 222);
    }

    #[test]
    fn friend_nudge_self_receive_targets_user() {
        let e = translate_event(
            ev(EventKind::FriendNudge {
                user_id: 42,
                is_self_send: false,
                is_self_receive: true,
                display_action: "".into(),
                display_suffix: "".into(),
                display_action_img_url: "".into(),
            }),
            99,
        )
        .unwrap();
        assert_eq!(e.kind, OutboundKind::PokePrivate);
        assert_eq!(e.user_id, 42);
        assert_eq!(e.target_id, 42);
    }

    #[test]
    fn friend_nudge_other_targets_self() {
        let e = translate_event(
            ev(EventKind::FriendNudge {
                user_id: 42,
                is_self_send: false,
                is_self_receive: false,
                display_action: "".into(),
                display_suffix: "".into(),
                display_action_img_url: "".into(),
            }),
            99,
        )
        .unwrap();
        assert_eq!(e.target_id, 99);
    }

    #[test]
    fn group_nudge_maps_directly() {
        let e = translate_event(
            ev(EventKind::GroupNudge {
                group_id: 1,
                sender_id: 2,
                receiver_id: 3,
                display_action: "".into(),
                display_suffix: "".into(),
                display_action_img_url: "".into(),
            }),
            99,
        )
        .unwrap();
        assert_eq!(e.kind, OutboundKind::PokeGroup);
        assert_eq!(e.group_id, 1);
        assert_eq!(e.user_id, 2);
        assert_eq!(e.target_id, 3);
    }

    #[test]
    fn group_recall_sets_group_id() {
        let e = translate_event(
            ev(EventKind::MessageRecall {
                message_scene: MessageScene::Group,
                peer_id: 444,
                message_seq: 11,
                sender_id: 22,
                operator_id: 33,
                display_suffix: "".into(),
            }),
            99,
        )
        .unwrap();
        assert_eq!(e.kind, OutboundKind::RecallGroup);
        assert_eq!(e.group_id, 444);
        assert_eq!(e.message_id, 11);
        assert_eq!(e.user_id, 22);
    }

    #[test]
    fn friend_recall_no_group_id() {
        let e = translate_event(
            ev(EventKind::MessageRecall {
                message_scene: MessageScene::Friend,
                peer_id: 777,
                message_seq: 12,
                sender_id: 88,
                operator_id: 88,
                display_suffix: "".into(),
            }),
            99,
        )
        .unwrap();
        assert_eq!(e.kind, OutboundKind::RecallPrivate);
        assert_eq!(e.group_id, 0);
        assert_eq!(e.message_id, 12);
    }

    #[test]
    fn unhandled_event_returns_none() {
        let e = translate_event(ev(EventKind::BotOffline { reason: "x".into() }), 99);
        assert!(e.is_none());
    }
}
