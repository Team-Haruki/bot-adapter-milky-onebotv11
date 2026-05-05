# AGENTS.md

This file provides guidance to Codex and other AI coding agents working in this repository.

## Project

Rust bridge that adapts an upstream Milky gateway to the OneBot-11 protocol. Downstream bots speak OneBot-11 to this bridge; the bridge speaks Milky upstream via [`milky-rust-sdk`](https://crates.io/crates/milky-rust-sdk). The README is in Chinese.

Single binary crate, edition 2024. MSRV is the latest stable Rust (`dtolnay/rust-toolchain@stable`). Docker images target Alpine Linux (musl libc).

## Setup

```bash
# No extra tools needed beyond a stable Rust toolchain
cargo build
```

## Common commands

```bash
# Run from a config file
cargo run -- --config config.json

# Test the whole crate
cargo test

# Run a single module's tests
cargo test bridge::

# Run a single test by name
cargo test parse_cq_string_message

# Format / lint (CI enforces both as errors)
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings

# Production build
cargo build --release --locked
# binary: target/release/milky-ob11-bridge

# Docker build (requires Docker)
docker build -t milky-ob11-bridge .
```

CI (`.github/workflows/ci.yml`) runs on every push to `main` and on every PR: `cargo fmt --all --check`, `cargo clippy ... -D warnings`, `cargo test --locked`.

The release workflow (`release-build.yml`) cross-compiles for linux/windows/darwin × amd64/arm64 on release publish.

The Docker workflow (`docker-build.yml`) builds Alpine images for linux/amd64 and linux/arm64 using native runners and pushes a multi-arch manifest to GHCR.

## Architecture

`src/main.rs` wires the layers together. The bridge is a single long-running async process (tokio multi-thread runtime) with three layers connected via channels:

1. **`src/milky/`** — upstream client. `Client::new` constructs a `milky_rust_sdk::MilkyClient` over WebSocket. `connect()` opens the WS, fetches login info, and spawns a translator task that converts SDK events into `types::InboundEvent`s. Outgoing methods (`send_*_message`, `delete_message`, `get_*`, `handle_*_request`) call the SDK plus segment-IR conversion (`segments.rs`).

2. **`src/bridge/`** — translation core. `Service::run` is the central event loop: pulls `InboundEvent`s, translates them via `translator::translate_event` into OneBot-11 payloads, and broadcasts via `onebot::Server::broadcast`. `handle_api` is the dispatch table for every supported OneBot-11 action. `message_ir.rs` is the intermediate representation for segment conversion, including CQ-code encode/decode.

3. **`src/onebot/`** — downstream surface (axum 0.8):
   - Forward WS: `/` (universal), `/api`, `/api/` (receive only), `/event`, `/event/` (send only).
   - HTTP API: `POST /http/<action>` when `enable_http_api` is set.
   - Reverse WS: dials out to configured URLs with auto-reconnect; sends `X-Self-ID`, `X-Client-Role`, optional `Authorization: Bearer <token>` headers.

**State (`src/state/`)**:
- `MessageMap` — LRU map of OneBot `message_id` → Milky message ref.
- `RequestMap` — `flag` strings → friend/group request refs.
- `Runtime` — login info + online/good status + heartbeat.

**Config (`src/config.rs`)**: strict JSON decoding (`#[serde(deny_unknown_fields)]`); unknown keys fail loading.

**Shutdown**: `tokio::sync::watch::channel<bool>`; `main` listens for SIGINT/SIGTERM (Unix) or Ctrl+C (Windows).

### Adding a new OneBot-11 action

1. Add a `match` arm to `Service::handle_api` in `src/bridge/service.rs`.
2. Define a params struct (with `#[serde(default)]` on every field) and decode via the `decode::<T>(params, &echo)` helper.
3. Call into `self.upstream`; add a method to `milky::Client` and the `Upstream` trait if needed.
4. Return `success(...)` or `failure(...)`. Unsupported actions fall through to `failure(1503, unsupported_action(...))`.

### Adding a new inbound event

1. Add a variant to `types::EventKind` in `src/types.rs`.
2. Emit it from `milky::events::translate_event` in `src/milky/events.rs`.
3. Translate it in `bridge::translator::translate_event` (`src/bridge/translator.rs`).

## Conventions

- **Logging**: `tracing` everywhere, initialized via `logging::init`. Use structured key/value pairs (`tracing::info!(field = %value, "msg")`), not format strings. Fields `bot_id`/`group_id`/`user_id` are treated as identity tags.
- **Error codes** returned to OneBot clients: `1400` (bad params), `1500` (upstream/unknown), `1502` (not found), `1503` (unsupported).
- **Errors**: propagate via `MilkyClientError` (`thiserror`-derived) with `#[from]`. Do not introduce `anyhow`.
- **`Upstream` trait**: mirrors `milky::Client`'s public async methods for test stubbing. Add new methods to both the trait and `StubUpstream` when extending the client.
- **Tests**: in-crate (`#[cfg(test)] mod tests`), synchronous where possible; async tests use `#[tokio::test]`. No real network connections in tests.

## Git Commits

All commit subjects must follow:

```text
[Type] Short description starting with capital letter
```

| Type      | Usage                                                 |
|-----------|-------------------------------------------------------|
| `[Feat]`  | New feature or capability                             |
| `[Fix]`   | Bug fix                                               |
| `[Chore]` | Maintenance, refactoring, dependency or build changes |
| `[Docs]`  | Documentation-only changes                            |

Rules:
- Description starts with a capital letter, imperative mood (`Add`, not `Added`).
- No trailing period. Subject ≤ 70 characters.
- Agent attribution via `Co-authored-by:` trailer:
  - Codex: `Co-authored-by: Codex <noreply@openai.com>`
  - Claude Sonnet 4.6: `Co-authored-by: Claude Sonnet 4.6 <noreply@anthropic.com>`
  - Copilot: `Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>`

Example:

```text
[Feat] Add Docker Alpine image and multi-arch build workflow

Co-authored-by: Codex <noreply@openai.com>
```
