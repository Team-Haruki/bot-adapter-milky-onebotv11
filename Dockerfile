# syntax=docker/dockerfile:1

# ── Builder ────────────────────────────────────────────────────────────────────
FROM rust:alpine AS builder

RUN apk add --no-cache musl-dev openssl-dev

ENV OPENSSL_STATIC=1

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src/ ./src/

RUN cargo build --release --locked

# ── Runtime ────────────────────────────────────────────────────────────────────
FROM alpine:3.23

RUN apk add --no-cache ca-certificates tzdata

WORKDIR /app

COPY --from=builder /build/target/release/milky-ob11-bridge /usr/local/bin/milky-ob11-bridge
COPY config.example.json ./config.example.json

VOLUME ["/app"]
EXPOSE 6700

ENTRYPOINT ["milky-ob11-bridge"]
CMD ["--config", "/app/config.json"]
