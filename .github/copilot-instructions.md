# GitHub Copilot Instructions

This is a Rust bridge between the Milky gateway protocol and OneBot-11. Single binary crate, edition 2024, using tokio async runtime.

## Key commands

```bash
cargo test                                               # run all tests
cargo fmt --all                                          # format
cargo clippy --all-targets --all-features -- -D warnings # lint
cargo build --release --locked                           # production build
```

## Project layout

| Path | Purpose |
|------|---------|
| `src/milky/` | Upstream Milky SDK client and event translation |
| `src/bridge/` | Core translation layer; `service.rs` dispatches OneBot-11 actions |
| `src/onebot/` | Downstream axum HTTP/WebSocket server |
| `src/state/` | In-memory state: message map, request map, runtime info |
| `src/config.rs` | Strict JSON config (`deny_unknown_fields`) |
| `src/logging.rs` | Custom colored tracing formatter |
| `src/types.rs` | Shared event and segment types |

## Coding conventions

- Use `tracing` for all logging with structured key/value fields — no format strings in log macros.
- Error handling uses `thiserror`-derived types; never introduce `anyhow`.
- OneBot-11 error codes: `1400` bad params, `1500` upstream error, `1502` not found, `1503` unsupported.
- When adding a new `milky::Client` method consumed by the bridge, also add it to the `Upstream` trait in `src/bridge/service.rs` and to `StubUpstream` in the tests module.
- Tests live in `#[cfg(test)] mod tests` blocks in the same file. No real network connections — use `StubUpstream` and the `Stub` handler.
- Config structs use `#[serde(default)]` on optional fields and `#[serde(deny_unknown_fields)]` on top-level types.

## Commit message format

```
[Type] Short imperative description
```

Types: `[Feat]`, `[Fix]`, `[Chore]`, `[Docs]`. No trailing period. ≤ 70 chars.
