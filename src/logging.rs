use std::fmt;

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::fmt::FmtContext;
use tracing_subscriber::fmt::format::{FormatEvent, FormatFields, Writer};
use tracing_subscriber::registry::LookupSpan;

const COLOR_GREEN: &str = "\x1b[32m";
const COLOR_BLUE: &str = "\x1b[34m";
const COLOR_MAGENTA: &str = "\x1b[35m";
const COLOR_YELLOW: &str = "\x1b[33m";
const COLOR_RED: &str = "\x1b[31m";
const COLOR_CYAN: &str = "\x1b[36m";
const COLOR_WHITE: &str = "\x1b[37m";
const COLOR_DARK_ORANGE: &str = "\x1b[38;5;208m";
const COLOR_CYAN1: &str = "\x1b[38;5;51m";
const COLOR_DARK_SLATE_GRAY1: &str = "\x1b[38;5;123m";
const COLOR_BRIGHT_BLUE: &str = "\x1b[94m";
const COLOR_BRIGHT_MAGENTA: &str = "\x1b[95m";
const COLOR_BRIGHT_CYAN: &str = "\x1b[96m";
const COLOR_RESET: &str = "\x1b[0m";

pub fn init(level: &str) {
    #[cfg(windows)]
    let _ = enable_ansi_support::enable_ansi_support();

    let level = normalize_level(level);
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        tracing_subscriber::EnvFilter::new(format!(
            "milky_ob11_bridge={level},milky_rust_sdk={level},warn"
        ))
    });
    let _ = tracing_subscriber::fmt()
        .event_format(ColoredFormatter)
        .with_env_filter(env_filter)
        .try_init();
}

fn normalize_level(level: &str) -> &'static str {
    match level.trim().to_ascii_lowercase().as_str() {
        "trace" => "trace",
        "debug" => "debug",
        "warn" | "warning" => "warn",
        "error" => "error",
        _ => "info",
    }
}

struct ColoredFormatter;

impl<S, N> FormatEvent<S, N> for ColoredFormatter
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        _ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let metadata = event.metadata();
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let level = level_name(metadata.level());
        let level_color = level_color(metadata.level());
        let component = component_name(metadata.target());
        let component_color = component_color(component);
        let mut visitor = EventVisitor::default();
        event.record(&mut visitor);

        let identity_tags = visitor.identity_tags();
        let after_component = if identity_tags.is_empty() { " " } else { "" };
        let after_identity = if identity_tags.is_empty() { "" } else { " " };
        let fields = if visitor.fields.is_empty() {
            String::new()
        } else {
            format!(" {}", visitor.fields.join(" "))
        };
        let message = format!(
            "{}{}{}{}",
            COLOR_WHITE,
            visitor.message.unwrap_or_default(),
            fields,
            COLOR_RESET
        );

        writeln!(
            writer,
            "{}[{}]{}[{}{}{}][{}{}{}]{}{}{}{}",
            COLOR_DARK_SLATE_GRAY1,
            now,
            COLOR_RESET,
            level_color,
            level,
            COLOR_RESET,
            component_color,
            component,
            COLOR_RESET,
            after_component,
            identity_tags,
            after_identity,
            message
        )
    }
}

#[derive(Default)]
struct EventVisitor {
    message: Option<String>,
    bot_id: Option<String>,
    group_id: Option<String>,
    user_id: Option<String>,
    fields: Vec<String>,
}

impl EventVisitor {
    fn record_value(&mut self, field: &Field, value: String) {
        match field.name() {
            "message" => {
                self.message = Some(value);
            }
            "bot_id" | "self_id" => {
                self.bot_id = Some(value);
            }
            "group_id" if !value.trim().is_empty() => {
                self.group_id = Some(value);
            }
            "user_id" => {
                self.user_id = Some(value);
            }
            "log_message" => {
                self.fields.push(format!("message={value}"));
            }
            _ => {
                self.fields.push(format!("{}={}", field.name(), value));
            }
        }
    }

    fn identity_tags(&self) -> String {
        let mut tags = String::new();
        if let Some(bot_id) = &self.bot_id {
            tags.push_str(&format!(
                "[{}Bot-{}{}]",
                COLOR_BRIGHT_MAGENTA, bot_id, COLOR_RESET
            ));
        }
        if let Some(group_id) = &self.group_id {
            tags.push_str(&format!(
                "[{}Group-{}{}]",
                COLOR_BLUE, group_id, COLOR_RESET
            ));
        }
        if let Some(user_id) = &self.user_id {
            tags.push_str(&format!("[{}User-{}{}]", COLOR_CYAN1, user_id, COLOR_RESET));
        }
        tags
    }
}

impl Visit for EventVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.record_value(field, value.to_string());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.record_value(field, value.to_string());
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.record_value(field, value.to_string());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.record_value(field, value.to_string());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.record_value(field, format!("{value:?}"));
    }
}

fn level_name(level: &Level) -> &'static str {
    match *level {
        Level::TRACE => "TRACE",
        Level::DEBUG => "DEBUG",
        Level::INFO => "INFO",
        Level::WARN => "WARNING",
        Level::ERROR => "ERROR",
    }
}

fn level_color(level: &Level) -> &'static str {
    match *level {
        Level::TRACE => COLOR_MAGENTA,
        Level::DEBUG => COLOR_BLUE,
        Level::INFO => COLOR_GREEN,
        Level::WARN => COLOR_DARK_ORANGE,
        Level::ERROR => COLOR_RED,
    }
}

fn component_name(target: &str) -> &str {
    let mut parts = target.split("::");
    match parts.next() {
        Some("milky_ob11_bridge") => parts.next().unwrap_or("main"),
        Some("milky_rust_sdk") => "sdk",
        Some(component) => component,
        None => "main",
    }
}

fn component_color(component: &str) -> &'static str {
    match component {
        "main" => COLOR_BRIGHT_CYAN,
        "bridge" => COLOR_GREEN,
        "milky" => COLOR_MAGENTA,
        "onebot" => COLOR_BRIGHT_BLUE,
        "state" => COLOR_CYAN,
        "config" => COLOR_BRIGHT_MAGENTA,
        "sdk" => COLOR_YELLOW,
        _ => COLOR_BRIGHT_CYAN,
    }
}
