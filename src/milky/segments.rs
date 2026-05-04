use std::collections::BTreeMap;

use milky_rust_sdk::prelude::{
    ImageData, IncomingSegment, MentionAllData, MentionData, OutgoingSegment, RecordData,
    ReplyData, TextData,
};

use super::error::MilkyClientError;
use crate::types::{Segment, SegmentType};

pub fn to_outgoing(segments: Vec<Segment>) -> Result<Vec<OutgoingSegment>, MilkyClientError> {
    let mut out = Vec::with_capacity(segments.len());
    for seg in segments {
        match seg.kind {
            SegmentType::Text => {
                let text = seg.data.get("text").cloned().unwrap_or_default();
                out.push(OutgoingSegment::Text(TextData { text }));
            }
            SegmentType::At => {
                let qq = seg.data.get("qq").map(String::as_str).unwrap_or("");
                if qq == "all" {
                    out.push(OutgoingSegment::MentionAll(MentionAllData));
                } else {
                    let user_id: i64 = qq.parse().map_err(|_| {
                        MilkyClientError::BadSegment(format!("at: invalid qq {qq:?}"))
                    })?;
                    out.push(OutgoingSegment::Mention(MentionData { user_id }));
                }
            }
            SegmentType::Reply => {
                let id = seg.data.get("id").map(String::as_str).unwrap_or("");
                let message_seq: i64 = id.parse().map_err(|_| {
                    MilkyClientError::BadSegment(format!("reply: invalid id {id:?}"))
                })?;
                out.push(OutgoingSegment::Reply(ReplyData { message_seq }));
            }
            SegmentType::Image => {
                let uri = seg
                    .data
                    .get("url")
                    .or_else(|| seg.data.get("file"))
                    .cloned()
                    .ok_or_else(|| {
                        MilkyClientError::BadSegment("image: missing url/file".into())
                    })?;
                out.push(OutgoingSegment::Image(ImageData {
                    uri,
                    summary: None,
                    sub_type: "normal".to_string(),
                }));
            }
            SegmentType::Record => {
                let uri = seg
                    .data
                    .get("url")
                    .or_else(|| seg.data.get("file"))
                    .cloned()
                    .ok_or_else(|| {
                        MilkyClientError::BadSegment("record: missing url/file".into())
                    })?;
                out.push(OutgoingSegment::Record(RecordData { uri }));
            }
            SegmentType::Poke => {
                // Milky has no outgoing poke segment in messages — skip silently.
            }
            SegmentType::File | SegmentType::Unknown => {
                if let Some(text) = seg.data.get("text").cloned() {
                    out.push(OutgoingSegment::Text(TextData { text }));
                }
            }
        }
    }
    Ok(out)
}

pub fn from_incoming(segments: Vec<IncomingSegment>) -> Vec<Segment> {
    segments.into_iter().map(from_incoming_one).collect()
}

fn from_incoming_one(seg: IncomingSegment) -> Segment {
    let mut data = BTreeMap::new();
    let kind = match seg {
        IncomingSegment::Text { text } => {
            data.insert("text".into(), text);
            SegmentType::Text
        }
        IncomingSegment::Mention { user_id } => {
            data.insert("qq".into(), user_id.to_string());
            SegmentType::At
        }
        IncomingSegment::MentionAll {} => {
            data.insert("qq".into(), "all".into());
            SegmentType::At
        }
        IncomingSegment::Reply { message_seq } => {
            data.insert("id".into(), message_seq.to_string());
            SegmentType::Reply
        }
        IncomingSegment::Image { temp_url, .. } => {
            data.insert("url".into(), temp_url);
            SegmentType::Image
        }
        IncomingSegment::Record { temp_url, .. } => {
            data.insert("url".into(), temp_url);
            SegmentType::Record
        }
        IncomingSegment::Face { face_id } => {
            data.insert("text".into(), format!("[unsupported:face:{face_id}]"));
            SegmentType::Unknown
        }
        IncomingSegment::Video { .. } => {
            data.insert("text".into(), "[unsupported:video]".into());
            SegmentType::Unknown
        }
        IncomingSegment::File { file_name, .. } => {
            data.insert("text".into(), format!("[unsupported:file:{file_name}]"));
            SegmentType::Unknown
        }
        IncomingSegment::Forward { forward_id } => {
            data.insert("text".into(), format!("[unsupported:forward:{forward_id}]"));
            SegmentType::Unknown
        }
        IncomingSegment::MarketFace { url } => {
            data.insert("text".into(), format!("[unsupported:market_face:{url}]"));
            SegmentType::Unknown
        }
        IncomingSegment::LightApp { app_name, .. } => {
            data.insert("text".into(), format!("[unsupported:light_app:{app_name}]"));
            SegmentType::Unknown
        }
        IncomingSegment::XML { service_id, .. } => {
            data.insert("text".into(), format!("[unsupported:xml:{service_id}]"));
            SegmentType::Unknown
        }
    };
    Segment {
        kind,
        data,
        raw: BTreeMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(kind: SegmentType, pairs: &[(&str, &str)]) -> Segment {
        let mut data = BTreeMap::new();
        for (k, v) in pairs {
            data.insert((*k).to_string(), (*v).to_string());
        }
        Segment {
            kind,
            data,
            raw: BTreeMap::new(),
        }
    }

    #[test]
    fn text_outgoing_roundtrips() {
        let out = to_outgoing(vec![seg(SegmentType::Text, &[("text", "hi")])]).unwrap();
        match &out[0] {
            OutgoingSegment::Text(t) => assert_eq!(t.text, "hi"),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn at_numeric_becomes_mention() {
        let out = to_outgoing(vec![seg(SegmentType::At, &[("qq", "12345")])]).unwrap();
        match &out[0] {
            OutgoingSegment::Mention(m) => assert_eq!(m.user_id, 12345),
            _ => panic!("expected mention"),
        }
    }

    #[test]
    fn at_all_becomes_mention_all() {
        let out = to_outgoing(vec![seg(SegmentType::At, &[("qq", "all")])]).unwrap();
        assert!(matches!(out[0], OutgoingSegment::MentionAll(_)));
    }

    #[test]
    fn at_invalid_qq_errors() {
        let err = to_outgoing(vec![seg(SegmentType::At, &[("qq", "nope")])]).unwrap_err();
        assert!(matches!(err, MilkyClientError::BadSegment(_)));
    }

    #[test]
    fn reply_parses_id() {
        let out = to_outgoing(vec![seg(SegmentType::Reply, &[("id", "789")])]).unwrap();
        match &out[0] {
            OutgoingSegment::Reply(r) => assert_eq!(r.message_seq, 789),
            _ => panic!("expected reply"),
        }
    }

    #[test]
    fn image_prefers_url_over_file() {
        let out = to_outgoing(vec![seg(
            SegmentType::Image,
            &[("url", "http://x/a.png"), ("file", "ignored")],
        )])
        .unwrap();
        match &out[0] {
            OutgoingSegment::Image(i) => {
                assert_eq!(i.uri, "http://x/a.png");
                assert_eq!(i.sub_type, "normal");
            }
            _ => panic!("expected image"),
        }
    }

    #[test]
    fn image_falls_back_to_file() {
        let out = to_outgoing(vec![seg(
            SegmentType::Image,
            &[("file", "file:///tmp/a.png")],
        )])
        .unwrap();
        match &out[0] {
            OutgoingSegment::Image(i) => assert_eq!(i.uri, "file:///tmp/a.png"),
            _ => panic!("expected image"),
        }
    }

    #[test]
    fn image_missing_uri_errors() {
        let err = to_outgoing(vec![seg(SegmentType::Image, &[])]).unwrap_err();
        assert!(matches!(err, MilkyClientError::BadSegment(_)));
    }

    #[test]
    fn record_uses_url_then_file() {
        let out = to_outgoing(vec![seg(
            SegmentType::Record,
            &[("file", "file:///r.silk")],
        )])
        .unwrap();
        match &out[0] {
            OutgoingSegment::Record(r) => assert_eq!(r.uri, "file:///r.silk"),
            _ => panic!("expected record"),
        }
    }

    #[test]
    fn poke_is_skipped() {
        let out = to_outgoing(vec![seg(SegmentType::Poke, &[("qq", "1")])]).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn unknown_with_text_falls_back_to_text() {
        let out = to_outgoing(vec![seg(SegmentType::Unknown, &[("text", "raw")])]).unwrap();
        match &out[0] {
            OutgoingSegment::Text(t) => assert_eq!(t.text, "raw"),
            _ => panic!("expected text fallback"),
        }
    }

    #[test]
    fn incoming_text_maps_to_text() {
        let out = from_incoming(vec![IncomingSegment::Text { text: "hi".into() }]);
        assert_eq!(out[0].kind, SegmentType::Text);
        assert_eq!(out[0].data.get("text").unwrap(), "hi");
    }

    #[test]
    fn incoming_mention_maps_to_at() {
        let out = from_incoming(vec![IncomingSegment::Mention { user_id: 999 }]);
        assert_eq!(out[0].kind, SegmentType::At);
        assert_eq!(out[0].data.get("qq").unwrap(), "999");
    }

    #[test]
    fn incoming_mention_all_maps_to_at_all() {
        let out = from_incoming(vec![IncomingSegment::MentionAll {}]);
        assert_eq!(out[0].kind, SegmentType::At);
        assert_eq!(out[0].data.get("qq").unwrap(), "all");
    }

    #[test]
    fn incoming_image_uses_temp_url() {
        let out = from_incoming(vec![IncomingSegment::Image {
            resource_id: "r".into(),
            temp_url: "https://t/img".into(),
            width: 1,
            height: 1,
            summary: "".into(),
            sub_type: "normal".into(),
        }]);
        assert_eq!(out[0].kind, SegmentType::Image);
        assert_eq!(out[0].data.get("url").unwrap(), "https://t/img");
    }

    #[test]
    fn incoming_face_falls_back_to_unknown() {
        let out = from_incoming(vec![IncomingSegment::Face {
            face_id: "42".into(),
        }]);
        assert_eq!(out[0].kind, SegmentType::Unknown);
        assert!(out[0].data.get("text").unwrap().contains("face:42"));
    }
}
