use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use crate::types::{Segment, SegmentType};

#[derive(Debug, thiserror::Error)]
pub enum MessageIrError {
    #[error("message is required")]
    Empty,
    #[error("decode message: {0}")]
    Decode(#[from] serde_json::Error),
    #[error("message segment must be object")]
    SegmentNotObject,
    #[error("unsupported message type")]
    Unsupported,
}

pub fn parse_onebot_message(
    raw: &Value,
    auto_escape: bool,
) -> Result<Vec<Segment>, MessageIrError> {
    match raw {
        Value::Null => Err(MessageIrError::Empty),
        Value::String(s) => {
            if auto_escape {
                Ok(vec![text_segment(s.clone())])
            } else {
                Ok(parse_cq_message(s))
            }
        }
        Value::Object(obj) => Ok(parse_segment_object(obj)),
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                let Some(obj) = item.as_object() else {
                    return Err(MessageIrError::SegmentNotObject);
                };
                out.extend(parse_segment_object(obj));
            }
            Ok(out)
        }
        _ => Err(MessageIrError::Unsupported),
    }
}

fn parse_segment_object(obj: &Map<String, Value>) -> Vec<Segment> {
    let segment_type = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let mut data: BTreeMap<String, String> = BTreeMap::new();
    if let Some(raw_data) = obj.get("data").and_then(|v| v.as_object()) {
        for (k, v) in raw_data {
            data.insert(k.clone(), value_to_string(v));
        }
    }
    match segment_type {
        "text" => vec![Segment {
            kind: SegmentType::Text,
            data: single_pair("text", data.get("text").cloned().unwrap_or_default()),
            raw: BTreeMap::new(),
        }],
        "image" => vec![Segment {
            kind: SegmentType::Image,
            data: normalize_media_data(&data),
            raw: BTreeMap::new(),
        }],
        "record" => vec![Segment {
            kind: SegmentType::Record,
            data: normalize_media_data(&data),
            raw: BTreeMap::new(),
        }],
        "at" => vec![Segment {
            kind: SegmentType::At,
            data: single_pair("qq", data.get("qq").cloned().unwrap_or_default()),
            raw: BTreeMap::new(),
        }],
        "reply" => vec![Segment {
            kind: SegmentType::Reply,
            data: single_pair("id", data.get("id").cloned().unwrap_or_default()),
            raw: BTreeMap::new(),
        }],
        "poke" => vec![Segment {
            kind: SegmentType::Poke,
            data: single_pair("qq", data.get("qq").cloned().unwrap_or_default()),
            raw: BTreeMap::new(),
        }],
        other => {
            let mut text = format!("[CQ:{other}");
            if !data.is_empty() {
                let parts: Vec<String> = data.iter().map(|(k, v)| format!("{k}={v}")).collect();
                text.push(',');
                text.push_str(&parts.join(","));
            }
            text.push(']');
            vec![text_segment(text)]
        }
    }
}

fn parse_cq_message(input: &str) -> Vec<Segment> {
    let mut segments: Vec<Segment> = Vec::new();
    let mut cursor = input;
    while !cursor.is_empty() {
        let Some(start) = cursor.find("[CQ:") else {
            segments.push(text_segment(unescape_cq_text(cursor)));
            break;
        };
        if start > 0 {
            segments.push(text_segment(unescape_cq_text(&cursor[..start])));
        }
        let after = &cursor[start..];
        let Some(end) = after.find(']') else {
            segments.push(text_segment(unescape_cq_text(after)));
            break;
        };
        let token = &after[4..end];
        segments.extend(parse_cq_token(token));
        cursor = &after[end + 1..];
    }
    segments
}

fn parse_cq_token(token: &str) -> Vec<Segment> {
    let mut parts = token.split(',');
    let Some(name) = parts.next() else {
        return Vec::new();
    };
    let mut data: BTreeMap<String, String> = BTreeMap::new();
    for item in parts {
        if let Some((k, v)) = item.split_once('=') {
            data.insert(k.to_string(), unescape_cq_text(v));
        }
    }
    match name {
        "image" => vec![Segment {
            kind: SegmentType::Image,
            data: normalize_media_data(&data),
            raw: BTreeMap::new(),
        }],
        "record" => vec![Segment {
            kind: SegmentType::Record,
            data: normalize_media_data(&data),
            raw: BTreeMap::new(),
        }],
        "at" => vec![Segment {
            kind: SegmentType::At,
            data: single_pair("qq", data.get("qq").cloned().unwrap_or_default()),
            raw: BTreeMap::new(),
        }],
        "reply" => vec![Segment {
            kind: SegmentType::Reply,
            data: single_pair("id", data.get("id").cloned().unwrap_or_default()),
            raw: BTreeMap::new(),
        }],
        "poke" => vec![Segment {
            kind: SegmentType::Poke,
            data: single_pair("qq", data.get("qq").cloned().unwrap_or_default()),
            raw: BTreeMap::new(),
        }],
        _ => vec![text_segment(format!("[CQ:{token}]"))],
    }
}

fn normalize_media_data(data: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let file = data.get("file").cloned().unwrap_or_default();
    let url = data.get("url").cloned().unwrap_or_default();
    let url = if url.is_empty() { file.clone() } else { url };
    let mut out = BTreeMap::new();
    out.insert("file".to_string(), file);
    out.insert("url".to_string(), url);
    out
}

pub fn build_onebot_message(format: &str, segments: &[Segment]) -> (Value, String) {
    if format == "string" {
        let raw = build_cq_string(segments);
        return (Value::String(raw.clone()), raw);
    }
    let array: Vec<Value> = segments
        .iter()
        .map(|seg| match seg.kind {
            SegmentType::Text => json!({
                "type": "text",
                "data": {"text": seg.data.get("text").cloned().unwrap_or_default()},
            }),
            SegmentType::Image => json!({
                "type": "image",
                "data": btreemap_to_json(&normalize_media_data(&seg.data)),
            }),
            SegmentType::Record => json!({
                "type": "record",
                "data": btreemap_to_json(&normalize_media_data(&seg.data)),
            }),
            SegmentType::At => json!({
                "type": "at",
                "data": {"qq": seg.data.get("qq").cloned().unwrap_or_default()},
            }),
            SegmentType::Reply => json!({
                "type": "reply",
                "data": {"id": seg.data.get("id").cloned().unwrap_or_default()},
            }),
            SegmentType::Poke => json!({
                "type": "poke",
                "data": {"qq": seg.data.get("qq").cloned().unwrap_or_default()},
            }),
            SegmentType::File | SegmentType::Unknown => json!({
                "type": "text",
                "data": {"text": seg.data.get("text").cloned().unwrap_or_default()},
            }),
        })
        .collect();
    (Value::Array(array), build_cq_string(segments))
}

pub fn build_cq_string(segments: &[Segment]) -> String {
    let mut buf = String::new();
    for seg in segments {
        match seg.kind {
            SegmentType::Text => {
                buf.push_str(&escape_cq_text(
                    seg.data.get("text").map(String::as_str).unwrap_or(""),
                ));
            }
            SegmentType::Image => {
                let file = pick_file_or_url(&seg.data);
                buf.push_str(&format!("[CQ:image,file={}]", escape_cq_value(&file)));
            }
            SegmentType::Record => {
                let file = pick_file_or_url(&seg.data);
                buf.push_str(&format!("[CQ:record,file={}]", escape_cq_value(&file)));
            }
            SegmentType::At => {
                let qq = seg.data.get("qq").cloned().unwrap_or_default();
                buf.push_str(&format!("[CQ:at,qq={}]", escape_cq_value(&qq)));
            }
            SegmentType::Reply => {
                let id = seg.data.get("id").cloned().unwrap_or_default();
                buf.push_str(&format!("[CQ:reply,id={}]", escape_cq_value(&id)));
            }
            SegmentType::Poke => {
                let qq = seg.data.get("qq").cloned().unwrap_or_default();
                buf.push_str(&format!("[CQ:poke,qq={}]", escape_cq_value(&qq)));
            }
            SegmentType::File | SegmentType::Unknown => {
                buf.push_str(&escape_cq_text(
                    seg.data.get("text").map(String::as_str).unwrap_or(""),
                ));
            }
        }
    }
    buf
}

fn pick_file_or_url(data: &BTreeMap<String, String>) -> String {
    let file = data.get("file").cloned().unwrap_or_default();
    if !file.is_empty() {
        return file;
    }
    data.get("url").cloned().unwrap_or_default()
}

fn escape_cq_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('[', "&#91;")
        .replace(']', "&#93;")
}

fn escape_cq_value(text: &str) -> String {
    escape_cq_text(text).replace(',', "&#44;")
}

fn unescape_cq_text(text: &str) -> String {
    text.replace("&#44;", ",")
        .replace("&#91;", "[")
        .replace("&#93;", "]")
        .replace("&amp;", "&")
}

fn text_segment(text: String) -> Segment {
    Segment {
        kind: SegmentType::Text,
        data: single_pair("text", text),
        raw: BTreeMap::new(),
    }
}

fn single_pair(key: &str, value: String) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    m.insert(key.to_string(), value);
    m
}

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

fn btreemap_to_json(data: &BTreeMap<String, String>) -> Value {
    let mut obj = Map::new();
    for (k, v) in data {
        obj.insert(k.clone(), Value::String(v.clone()));
    }
    Value::Object(obj)
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
    fn parse_array_message() {
        let raw = serde_json::json!([
            {"type":"text","data":{"text":"hello"}},
            {"type":"at","data":{"qq":"12345"}},
            {"type":"image","data":{"file":"http://example.com/a.png"}},
        ]);
        let segments = parse_onebot_message(&raw, false).unwrap();
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0].kind, SegmentType::Text);
        assert_eq!(segments[0].data.get("text").unwrap(), "hello");
        assert_eq!(segments[1].kind, SegmentType::At);
        assert_eq!(segments[1].data.get("qq").unwrap(), "12345");
        assert_eq!(segments[2].kind, SegmentType::Image);
        assert_eq!(
            segments[2].data.get("url").unwrap(),
            "http://example.com/a.png"
        );
    }

    #[test]
    fn parse_cq_string_message() {
        let raw = Value::String("[CQ:at,qq=42]abc[CQ:reply,id=100]".to_string());
        let segments = parse_onebot_message(&raw, false).unwrap();
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0].kind, SegmentType::At);
        assert_eq!(segments[0].data.get("qq").unwrap(), "42");
        assert_eq!(segments[1].kind, SegmentType::Text);
        assert_eq!(segments[1].data.get("text").unwrap(), "abc");
        assert_eq!(segments[2].kind, SegmentType::Reply);
        assert_eq!(segments[2].data.get("id").unwrap(), "100");
    }

    #[test]
    fn parse_string_with_auto_escape_keeps_literal() {
        let raw = Value::String("[CQ:at,qq=1]raw".to_string());
        let segments = parse_onebot_message(&raw, true).unwrap();
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].kind, SegmentType::Text);
        assert_eq!(segments[0].data.get("text").unwrap(), "[CQ:at,qq=1]raw");
    }

    #[test]
    fn parse_object_treated_as_single_segment() {
        let raw = serde_json::json!({"type": "text", "data": {"text": "hi"}});
        let segments = parse_onebot_message(&raw, false).unwrap();
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].kind, SegmentType::Text);
    }

    #[test]
    fn parse_unknown_segment_falls_back_to_cq_text() {
        let raw = serde_json::json!({
            "type": "face",
            "data": {"id": "7"},
        });
        let segments = parse_onebot_message(&raw, false).unwrap();
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].kind, SegmentType::Text);
        assert_eq!(segments[0].data.get("text").unwrap(), "[CQ:face,id=7]");
    }

    #[test]
    fn parse_empty_json_errors() {
        let raw = Value::Null;
        let err = parse_onebot_message(&raw, false).unwrap_err();
        assert!(matches!(err, MessageIrError::Empty));
    }

    #[test]
    fn build_string_format_escapes_text() {
        let (out, raw) = build_onebot_message(
            "string",
            &[
                seg(SegmentType::Text, &[("text", "a&b")]),
                seg(SegmentType::At, &[("qq", "7")]),
            ],
        );
        assert_eq!(out.as_str().unwrap(), "a&amp;b[CQ:at,qq=7]");
        assert_eq!(raw, "a&amp;b[CQ:at,qq=7]");
    }

    #[test]
    fn build_array_format_emits_objects() {
        let (out, raw) = build_onebot_message(
            "array",
            &[
                seg(SegmentType::Text, &[("text", "x")]),
                seg(SegmentType::Image, &[("file", "http://x/a.png")]),
            ],
        );
        let arr = out.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["type"], "text");
        assert_eq!(arr[0]["data"]["text"], "x");
        assert_eq!(arr[1]["type"], "image");
        assert_eq!(arr[1]["data"]["url"], "http://x/a.png");
        assert_eq!(arr[1]["data"]["file"], "http://x/a.png");
        assert!(raw.contains("[CQ:image"));
    }

    #[test]
    fn cq_token_with_escaped_comma_unescapes() {
        let raw = Value::String("[CQ:image,url=http://x/a&#44;b.png]".to_string());
        let segments = parse_onebot_message(&raw, false).unwrap();
        assert_eq!(segments[0].kind, SegmentType::Image);
        assert_eq!(segments[0].data.get("url").unwrap(), "http://x/a,b.png");
    }
}
