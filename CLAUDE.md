# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

Rust bridge that adapts an upstream Milky gateway to the OneBot-11 protocol. Downstream bots speak OneBot-11 to this bridge; the bridge speaks Milky upstream via [`milky-rust-sdk`](https://crates.io/crates/milky-rust-sdk). The README is in Chinese.

The repository was originally a Go implementation; the Go tree (`cmd/`, `internal/`, `go.mod`, `go.sum`) has been removed. Single binary crate, edition 2024, MSRV is the latest stable shipped on `dtolnay/rust-toolchain@stable`.

## Common commands

```bash
# Run from a config file
cargo run -- --config config.json
# or, after a build:
./target/debug/milky-ob11-bridge --config config.json

# Test the whole crate
cargo test

# Run a single module's tests
cargo test bridge::

# Run a single test by name
cargo test parse_cq_string_message

# Format / lint (CI enforces both as errors)
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings

# Production-style build (matches release-build.yml)
cargo build --release --locked
# binary lands at target/release/milky-ob11-bridge
```

CI runs on every push to `main`/`rust` and on every PR (`.github/workflows/ci.yml`): `cargo fmt --all --check`, `cargo clippy ... -D warnings`, `cargo test --locked`. The release workflow (`release-build.yml`) cross-compiles for linux/windows/darwin × amd64/arm64 on release publish; tests run only on the linux/amd64 matrix entry.

## Architecture

`src/main.rs` wires the layers together. The bridge is a single long-running async process (tokio multi-thread runtime) composed of three layers connected via channels:

1. **`src/milky/`** — upstream client. `Client::new` constructs a `milky_rust_sdk::MilkyClient` over WebSocket and an internal SDK event channel. `connect()` opens the WS, fetches login info, and spawns a translator task that pulls SDK events, runs them through `events::translate_event`, and forwards normalized `types::InboundEvent`s onto the bridge-supplied `mpsc::Sender`. Outgoing methods (`send_*_message`, `delete_message`, `get_*`, `handle_*_request`) are 1:1 with the SDK plus the segment-IR conversion in `segments.rs`.
2. **`src/bridge/`** — translation core. `Service::run` is the central event loop: it pulls `InboundEvent`s from the channel, translates them via `translator::translate_event` into OneBot-11 event payloads (`serde_json::Value`), and broadcasts them through `onebot::Server::broadcast`. It also implements the `onebot::Handler` trait — `handle_api` is the dispatch table for every supported OneBot-11 action (`send_*_msg`, `get_*`, `set_*_request`, `delete_msg`, `get_msg`, `get_login_info`, `get_status`, `get_version_info`, `can_send_*`). `message_ir.rs` is the intermediate representation that segment parsers/builders go through in both directions, including CQ-code encode/decode.
3. **`src/onebot/`** — downstream surface. `Server` exposes the OneBot-11 endpoints over HTTP/WS using axum 0.8:
   - Forward WS: `/` (universal, send+receive), `/api`, `/api/` (receive only), `/event`, `/event/` (send only).
   - HTTP API: `POST /http/<action>` (and `GET`) when `enable_http_api` is set.
   - Reverse WS: dialed out to `onebot.reverse.{url,api_url,event_url}` with auto-reconnect; sends `X-Self-ID`, `X-Client-Role`, optional `Authorization: Bearer <token>` headers per OneBot-11.

`Server` calls back into `bridge::Service` via the `onebot::Handler` trait (`handle_api`, `on_ws_connect`, `current_self_id`) — this is the seam between the layers. `Service::run` broadcasts to all connected forward + reverse WS clients flagged `can_send`.

**State (`src/state/`)** is in-memory only:
- `MessageMap` — LRU map of OneBot `message_id` → Milky message ref (used by `get_msg` / `delete_msg`).
- `RequestMap` — `flag` strings → friend/group request refs (used by `set_*_add_request`).
- `Runtime` — login info + online/good status used by `get_login_info`, `get_status`, and the periodic heartbeat event.

A heartbeat ticker in `Service::run` (driven by `bridge.heartbeat_interval_ms`) emits `meta_event` heartbeats to all connected clients.

**Config (`src/config.rs`)** uses strict JSON decoding (`#[serde(deny_unknown_fields)]`); unknown keys fail loading. `Config::load` reads, parses, then runs `validate()`. `bridge.message_format` must be `"array"` or `"string"` — array is the default and is what the IR aligns to.

**Shutdown** is coordinated through `tokio::sync::watch::channel<bool>`: `main` listens for SIGINT/SIGTERM (Unix) or Ctrl+C (Windows), then sends `true` to wake `server.run` and `service.run` simultaneously. After both join, `service.shutdown()` closes the SDK.

### Adding a new OneBot-11 action

1. Add a `match` arm to `Service::handle_api` in `src/bridge/service.rs` after the existing entries.
2. Define a params struct (with `#[serde(default)]` on every field) and decode via the `decode::<T>(params, &echo)` helper.
3. Call into `self.upstream` for any Milky-side work (add a method to `milky::Client` and the `Upstream` trait if needed). Translate the result to the OneBot-11 response shape and return via `success(...)` / `failure(...)`.
4. Unsupported actions fall through to `failure(1503, unsupported_action(...))` — keep that contract.

### Adding a new inbound event

1. Add a variant to `types::EventKind` in `src/types.rs`.
2. Emit it from `milky::events::translate_event` in `src/milky/events.rs`.
3. Translate it in `bridge::translator::translate_event` (`src/bridge/translator.rs`) — that's where it gets shaped into the OneBot-11 payload.

## Conventions observed in the codebase

- Logging is `tracing` everywhere, initialized once in `main.rs` via `logging::init` at the level set by `bridge.log_level`. The custom `ColoredFormatter` in `src/logging.rs` produces `[ts][LEVEL][component] message field=value` lines and recognizes `bot_id`/`group_id`/`user_id` fields as identity tags. Use structured key/value pairs (`tracing::info!(field = %value, "msg")`), not formatted strings. ANSI escape sequences are emitted unconditionally; `enable-ansi-support` enables VT processing on Windows so cmd renders colors.
- Errors returned to OneBot clients use the codes already in use: `1400` (bad params), `1500` (upstream/unknown), `1502` (not found), `1503` (unsupported).
- `MilkyClient` errors propagate via `MilkyClientError` (`thiserror`-derived) which wraps `milky_rust_sdk::MilkyError` with `#[from]`. Keep that pattern when adding new SDK calls — do not introduce `anyhow`.
- The `Upstream` trait in `bridge::service` mirrors `milky::Client`'s public async methods so tests can inject a stub. When you add a new `Client` method that the bridge consumes, also add it to `Upstream` (and the test stub in `bridge::service::tests::StubUpstream`).
- Tests are in-crate (`#[cfg(test)] mod tests` per file) and synchronous where possible; async tests use `#[tokio::test]`. Network-touching code is exercised through stubs (`StubUpstream`, `Stub` handler), not real connections.

## Git Commits

All commit subjects must follow:

```text
[Type] Short description starting with capital letter
```

Allowed types:

| Type      | Usage                                                 |
|-----------|-------------------------------------------------------|
| `[Feat]`  | New feature or capability                             |
| `[Fix]`   | Bug fix                                               |
| `[Chore]` | Maintenance, refactoring, dependency or build changes |
| `[Docs]`  | Documentation-only changes                            |

Rules:

- Description starts with a capital letter.
- Use imperative mood: `Add ...`, not `Added ...`.
- No trailing period.
- Keep the subject at or below roughly 70 characters.
- **Agent attribution uses the standard Git `Co-authored-by:` trailer in
  the commit body, not a free-form `Agent:` line.** This makes GitHub
  render the co-author avatar on the commit page. The trailer must be on
  its own line, separated from the subject by a blank line, in the form
  `Co-authored-by: <Display Name> <email>`. Suggested values per agent:
  - Claude (any 4.x): `Co-authored-by: Claude Opus 4.7 <noreply@anthropic.com>`
    (substitute the actual model, e.g. `Claude Sonnet 4.6`)
  - Codex: `Co-authored-by: Codex <noreply@openai.com>`
  - Copilot: `Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>`

Project examples:

```text
[Feat] Add colored tracing formatter with Windows ANSI support
[Fix] Default reverse websocket to universal single connection
[Chore] Migrate Go bridge implementation to Rust
[Docs] Document Milky ws_endpoint path constraint
```

Agent-authored commit example:

```text
[Docs] Add agent commit guidelines

Co-authored-by: Codex <noreply@openai.com>
```
