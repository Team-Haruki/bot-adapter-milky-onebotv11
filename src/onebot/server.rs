use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use axum::{
    Router,
    body::Bytes,
    extract::{
        Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, HeaderValue, StatusCode, header::AUTHORIZATION},
    response::{IntoResponse, Response},
    routing::{any, get},
};
use futures_util::{SinkExt, StreamExt};
use http::Request;
use serde_json::{Map, Value};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, mpsc, watch};
use tokio_tungstenite::tungstenite::{
    Message as TMessage, client::IntoClientRequest, http::HeaderValue as THeaderValue,
};

use crate::config::OneBotConfig;

use super::api::{ApiRequest, ApiResponse, failure};

#[async_trait]
pub trait Handler: Send + Sync + 'static {
    async fn handle_api(&self, req: ApiRequest) -> ApiResponse;
    async fn on_ws_connect(&self, role: &str) -> Vec<Value>;
    fn current_self_id(&self) -> i64;
}

pub struct Server {
    cfg: OneBotConfig,
    handler: Arc<dyn Handler>,
    clients: Mutex<HashMap<u64, ClientConn>>,
    next_id: AtomicU64,
}

#[derive(Clone)]
struct ClientConn {
    can_send: bool,
    out_tx: mpsc::UnboundedSender<String>,
}

impl Server {
    pub fn new(cfg: OneBotConfig, handler: Arc<dyn Handler>) -> Arc<Self> {
        Arc::new(Self {
            cfg,
            handler,
            clients: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        })
    }

    pub async fn run(self: Arc<Self>, mut shutdown: watch::Receiver<bool>) -> std::io::Result<()> {
        let addr: SocketAddr = format!("{}:{}", self.cfg.host, self.cfg.port)
            .parse()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        let listener = TcpListener::bind(addr).await?;
        tracing::info!(?addr, "onebot server listening");

        self.clone().spawn_reverse_clients(shutdown.clone());

        let app = self.clone().router();
        let mut sd = shutdown.clone();
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = sd.changed().await;
            })
            .await?;

        // Drain client list so writer tasks terminate.
        self.clients.lock().await.clear();
        let _ = shutdown.changed().await;
        Ok(())
    }

    pub async fn broadcast(&self, event: Value) {
        let payload = match serde_json::to_string(&event) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(err = %e, "marshal onebot event failed");
                return;
            }
        };
        let mut to_drop: Vec<u64> = Vec::new();
        let clients = self.clients.lock().await;
        for (id, c) in clients.iter() {
            if !c.can_send {
                continue;
            }
            if c.out_tx.send(payload.clone()).is_err() {
                to_drop.push(*id);
            }
        }
        drop(clients);
        if !to_drop.is_empty() {
            let mut clients = self.clients.lock().await;
            for id in to_drop {
                clients.remove(&id);
            }
        }
    }

    fn router(self: Arc<Self>) -> Router {
        Router::new()
            .route("/", any(handle_universal_ws))
            .route("/api", any(handle_api_ws))
            .route("/api/", any(handle_api_ws))
            .route("/event", any(handle_event_ws))
            .route("/event/", any(handle_event_ws))
            .route("/http", get(handle_http_root).post(handle_http_root))
            .route(
                "/http/{*action}",
                get(handle_http_api).post(handle_http_api),
            )
            .with_state(self)
    }

    fn is_authorized(&self, headers: &HeaderMap, query: &HashMap<String, String>) -> bool {
        let token = self.cfg.access_token.trim();
        if token.is_empty() {
            return true;
        }
        if let Some(q) = query.get("access_token").map(|s| s.trim())
            && q == token
        {
            return true;
        }
        let Some(auth) = headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()) else {
            return false;
        };
        let auth = auth.trim();
        let auth = auth
            .strip_prefix("Bearer ")
            .or_else(|| auth.strip_prefix("Token "))
            .unwrap_or(auth);
        auth == token
    }

    async fn add_client(&self, conn: ClientConn) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.clients.lock().await.insert(id, conn);
        id
    }

    async fn remove_client(&self, id: u64) {
        self.clients.lock().await.remove(&id);
    }

    fn spawn_reverse_clients(self: Arc<Self>, shutdown: watch::Receiver<bool>) {
        let cfg = self.cfg.reverse.clone();
        if !cfg.enable {
            return;
        }
        let api = cfg.api_url.trim();
        let event = cfg.event_url.trim();
        let universal = cfg.use_universal_client || (api.is_empty() && event.is_empty());

        if universal {
            let target = first_non_empty(&[&cfg.url, &cfg.api_url, &cfg.event_url]);
            if !target.is_empty() {
                tokio::spawn(self.clone().run_reverse_client(
                    target,
                    "Universal",
                    true,
                    true,
                    shutdown,
                ));
            }
            return;
        }

        let api_url = first_non_empty(&[&cfg.api_url, &cfg.url]);
        let event_url = first_non_empty(&[&cfg.event_url, &cfg.url]);
        if !api_url.is_empty() {
            tokio::spawn(self.clone().run_reverse_client(
                api_url,
                "API",
                false,
                true,
                shutdown.clone(),
            ));
        }
        if !event_url.is_empty() {
            tokio::spawn(
                self.clone()
                    .run_reverse_client(event_url, "Event", true, false, shutdown),
            );
        }
    }

    async fn run_reverse_client(
        self: Arc<Self>,
        target: String,
        role: &'static str,
        can_send: bool,
        can_receive: bool,
        mut shutdown: watch::Receiver<bool>,
    ) {
        let reconnect = Duration::from_millis(self.cfg.reverse.reconnect_interval_ms);
        loop {
            if *shutdown.borrow() {
                return;
            }
            match self.dial_reverse(&target, role).await {
                Ok(socket) => {
                    tracing::info!(target = %target, role, "reverse websocket connected");
                    self.clone()
                        .serve_reverse(socket, role, can_send, can_receive, shutdown.clone())
                        .await;
                }
                Err(e) => {
                    tracing::warn!(target = %target, role, err = %e, "reverse websocket connect failed");
                }
            }
            tokio::select! {
                _ = tokio::time::sleep(reconnect) => {}
                _ = shutdown.changed() => return,
            }
        }
    }

    async fn dial_reverse(
        &self,
        target: &str,
        role: &'static str,
    ) -> Result<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        ReverseDialError,
    > {
        let mut req: Request<()> = target
            .into_client_request()
            .map_err(|e| ReverseDialError::Other(e.to_string()))?;
        let headers = req.headers_mut();
        let self_id = self.handler.current_self_id();
        if self_id != 0 {
            headers.insert(
                "X-Self-ID",
                THeaderValue::from_str(&self_id.to_string())
                    .map_err(|e| ReverseDialError::Other(e.to_string()))?,
            );
        }
        headers.insert(
            "X-Client-Role",
            THeaderValue::from_static(match role {
                "Universal" => "Universal",
                "API" => "API",
                "Event" => "Event",
                _ => "Universal",
            }),
        );
        let token = self.cfg.access_token.trim();
        if !token.is_empty() {
            headers.insert(
                tokio_tungstenite::tungstenite::http::header::AUTHORIZATION,
                THeaderValue::from_str(&format!("Bearer {token}"))
                    .map_err(|e| ReverseDialError::Other(e.to_string()))?,
            );
        }
        let (socket, _resp) = tokio_tungstenite::connect_async(req)
            .await
            .map_err(|e| ReverseDialError::Other(e.to_string()))?;
        Ok(socket)
    }

    async fn serve_reverse(
        self: Arc<Self>,
        socket: tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        role: &'static str,
        can_send: bool,
        can_receive: bool,
        mut shutdown: watch::Receiver<bool>,
    ) {
        let (mut sink, mut stream) = socket.split();
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
        let id = self
            .add_client(ClientConn {
                can_send,
                out_tx: out_tx.clone(),
            })
            .await;

        if can_send {
            for ev in self
                .handler
                .on_ws_connect(&format!("reverse-{}", role.to_lowercase()))
                .await
            {
                if let Ok(s) = serde_json::to_string(&ev) {
                    let _ = out_tx.send(s);
                }
            }
        }

        let writer = tokio::spawn(async move {
            while let Some(payload) = out_rx.recv().await {
                if sink.send(TMessage::Text(payload.into())).await.is_err() {
                    break;
                }
            }
            let _ = sink.close().await;
        });

        loop {
            tokio::select! {
                _ = shutdown.changed() => break,
                msg = stream.next() => {
                    let Some(msg) = msg else { break };
                    let Ok(msg) = msg else { break };
                    match msg {
                        TMessage::Text(text) => {
                            if !can_receive { continue; }
                            self.dispatch_request(text.as_ref(), &out_tx).await;
                        }
                        TMessage::Binary(_) => continue,
                        TMessage::Ping(_) | TMessage::Pong(_) | TMessage::Frame(_) => continue,
                        TMessage::Close(_) => break,
                    }
                }
            }
        }

        drop(out_tx);
        let _ = writer.await;
        self.remove_client(id).await;
        tracing::info!(role, "reverse websocket disconnected");
    }

    async fn dispatch_request(&self, payload: &str, out_tx: &mpsc::UnboundedSender<String>) {
        tracing::debug!(payload = payload, "onebot api request");
        let resp = match serde_json::from_str::<ApiRequest>(payload) {
            Ok(req) => self.handler.handle_api(req).await,
            Err(_) => failure(1400, "invalid json request", None),
        };
        if let Ok(s) = serde_json::to_string(&resp) {
            tracing::debug!(payload = %s, "onebot api response");
            let _ = out_tx.send(s);
        }
    }
}

#[derive(Debug)]
enum ReverseDialError {
    Other(String),
}

impl std::fmt::Display for ReverseDialError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReverseDialError::Other(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for ReverseDialError {}

// ---- forward route handlers --------------------------------------------------

async fn handle_universal_ws(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    ws: WebSocketUpgrade,
) -> Response {
    if !server.cfg.enable_ws_universal {
        return (StatusCode::NOT_FOUND, "websocket universal mode disabled").into_response();
    }
    if !server.is_authorized(&headers, &query) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    ws.on_upgrade(move |socket| serve_forward(server, socket, "universal", true, true))
}

async fn handle_api_ws(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    ws: WebSocketUpgrade,
) -> Response {
    if !server.cfg.enable_ws_api {
        return (StatusCode::NOT_FOUND, "websocket api mode disabled").into_response();
    }
    if !server.is_authorized(&headers, &query) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    ws.on_upgrade(move |socket| serve_forward(server, socket, "api", false, true))
}

async fn handle_event_ws(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    ws: WebSocketUpgrade,
) -> Response {
    if !server.cfg.enable_ws_event {
        return (StatusCode::NOT_FOUND, "websocket event mode disabled").into_response();
    }
    if !server.is_authorized(&headers, &query) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    ws.on_upgrade(move |socket| serve_forward(server, socket, "event", true, false))
}

async fn serve_forward(
    server: Arc<Server>,
    socket: WebSocket,
    kind: &'static str,
    can_send: bool,
    can_receive: bool,
) {
    let (mut sink, mut stream) = socket.split();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();

    let id = server
        .add_client(ClientConn {
            can_send,
            out_tx: out_tx.clone(),
        })
        .await;

    if can_send {
        for ev in server.handler.on_ws_connect(kind).await {
            if let Ok(s) = serde_json::to_string(&ev) {
                let _ = out_tx.send(s);
            }
        }
    }

    let writer = tokio::spawn(async move {
        while let Some(payload) = out_rx.recv().await {
            if sink.send(Message::Text(payload.into())).await.is_err() {
                break;
            }
        }
        let _ = sink.close().await;
    });

    while let Some(Ok(msg)) = stream.next().await {
        match msg {
            Message::Text(text) => {
                if !can_receive {
                    continue;
                }
                tracing::debug!(payload = %text, "onebot api request");
                let resp = match serde_json::from_str::<ApiRequest>(text.as_ref()) {
                    Ok(req) => server.handler.handle_api(req).await,
                    Err(_) => failure(1400, "invalid json request", None),
                };
                if let Ok(s) = serde_json::to_string(&resp) {
                    tracing::debug!(payload = %s, "onebot api response");
                    if out_tx.send(s).is_err() {
                        break;
                    }
                }
            }
            Message::Binary(_) | Message::Ping(_) | Message::Pong(_) => continue,
            Message::Close(_) => break,
        }
    }

    drop(out_tx);
    let _ = writer.await;
    server.remove_client(id).await;
}

// ---- HTTP API ----------------------------------------------------------------

async fn handle_http_root(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    if !server.cfg.enable_http_api {
        return (StatusCode::NOT_FOUND, "").into_response();
    }
    if !server.is_authorized(&headers, &query) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    (StatusCode::BAD_REQUEST, "action is required").into_response()
}

async fn handle_http_api(
    State(server): State<Arc<Server>>,
    axum::extract::Path(action): axum::extract::Path<String>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    body: Bytes,
) -> Response {
    if !server.cfg.enable_http_api {
        return (StatusCode::NOT_FOUND, "").into_response();
    }
    if !server.is_authorized(&headers, &query) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    if action.is_empty() {
        return (StatusCode::BAD_REQUEST, "action is required").into_response();
    }

    let params = match build_http_params(&query, &body, !body.is_empty()) {
        Ok(p) => p,
        Err(msg) => return (StatusCode::BAD_REQUEST, msg).into_response(),
    };

    let req = ApiRequest {
        action,
        params,
        echo: None,
    };
    let resp = server.handler.handle_api(req).await;
    let mut response = match serde_json::to_string(&resp) {
        Ok(body) => (
            [(
                axum::http::header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            )],
            body,
        )
            .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    if resp.retcode == 1400 {
        *response.status_mut() = StatusCode::BAD_REQUEST;
    }
    response
}

fn build_http_params(
    query: &HashMap<String, String>,
    body: &Bytes,
    has_body: bool,
) -> Result<Option<Value>, String> {
    let mut params: Map<String, Value> = Map::new();
    for (key, value) in query {
        if key == "access_token" {
            continue;
        }
        params.insert(key.clone(), normalize_http_scalar(key, value));
    }
    if has_body {
        let trimmed = std::str::from_utf8(body)
            .map_err(|e| format!("invalid utf-8 body: {e}"))?
            .trim();
        if !trimmed.is_empty() {
            let payload: Map<String, Value> =
                serde_json::from_str(trimmed).map_err(|e| format!("invalid json body: {e}"))?;
            for (k, v) in payload {
                params.insert(k, v);
            }
        }
    }
    if params.is_empty() {
        Ok(None)
    } else {
        Ok(Some(Value::Object(params)))
    }
}

fn normalize_http_scalar(key: &str, value: &str) -> Value {
    let key = key.trim().to_ascii_lowercase();
    match key.as_str() {
        "message" => Value::String(value.to_string()),
        "user_id" | "group_id" | "message_id" | "self_id" | "duration" | "delay" | "times" => {
            if let Ok(n) = value.parse::<i64>() {
                Value::Number(n.into())
            } else {
                Value::String(value.to_string())
            }
        }
        "auto_escape" | "approve" | "enable" | "no_cache" | "reject_add_request" => {
            match value.to_ascii_lowercase().as_str() {
                "true" | "1" => Value::Bool(true),
                "false" | "0" => Value::Bool(false),
                _ => Value::String(value.to_string()),
            }
        }
        _ => Value::String(value.to_string()),
    }
}

fn first_non_empty(values: &[&str]) -> String {
    for v in values {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::OneBotConfig;

    struct Stub;

    #[async_trait]
    impl Handler for Stub {
        async fn handle_api(&self, _: ApiRequest) -> ApiResponse {
            super::super::api::success(Value::Null, None)
        }
        async fn on_ws_connect(&self, _: &str) -> Vec<Value> {
            Vec::new()
        }
        fn current_self_id(&self) -> i64 {
            123456
        }
    }

    fn server(token: &str) -> Arc<Server> {
        let cfg = OneBotConfig {
            access_token: token.to_string(),
            ..OneBotConfig::default()
        };
        Server::new(cfg, Arc::new(Stub))
    }

    #[test]
    fn first_non_empty_picks_first() {
        assert_eq!(first_non_empty(&["", "a", "b"]), "a");
        assert_eq!(first_non_empty(&["", "  ", "x"]), "x");
        assert_eq!(first_non_empty(&[]), "");
    }

    #[test]
    fn normalize_user_id_to_number() {
        match normalize_http_scalar("user_id", "123") {
            Value::Number(n) => assert_eq!(n.as_i64(), Some(123)),
            v => panic!("expected number, got {v:?}"),
        }
    }

    #[test]
    fn normalize_message_stays_string() {
        match normalize_http_scalar("message", "123") {
            Value::String(s) => assert_eq!(s, "123"),
            v => panic!("expected string, got {v:?}"),
        }
    }

    #[test]
    fn normalize_approve_to_bool() {
        match normalize_http_scalar("approve", "true") {
            Value::Bool(b) => assert!(b),
            v => panic!("expected bool, got {v:?}"),
        }
    }

    #[tokio::test]
    async fn authorized_via_query_token() {
        let s = server("abc");
        let mut q = HashMap::new();
        q.insert("access_token".into(), "abc".into());
        assert!(s.is_authorized(&HeaderMap::new(), &q));
    }

    #[tokio::test]
    async fn authorized_via_bearer_header() {
        let s = server("abc");
        let mut h = HeaderMap::new();
        h.insert(AUTHORIZATION, HeaderValue::from_static("Bearer abc"));
        assert!(s.is_authorized(&h, &HashMap::new()));
    }

    #[tokio::test]
    async fn unauthorized_without_token_match() {
        let s = server("abc");
        assert!(!s.is_authorized(&HeaderMap::new(), &HashMap::new()));
    }

    #[tokio::test]
    async fn no_token_means_authorized() {
        let s = server("");
        assert!(s.is_authorized(&HeaderMap::new(), &HashMap::new()));
    }

    #[test]
    fn http_params_query_only() {
        let mut q = HashMap::new();
        q.insert("user_id".into(), "42".into());
        q.insert("message".into(), "hi".into());
        let params = build_http_params(&q, &Bytes::new(), false)
            .unwrap()
            .unwrap();
        let obj = params.as_object().unwrap();
        assert_eq!(obj["user_id"], Value::Number(42.into()));
        assert_eq!(obj["message"], Value::String("hi".into()));
    }

    #[test]
    fn http_params_body_overrides_query() {
        let mut q = HashMap::new();
        q.insert("user_id".into(), "1".into());
        let body = Bytes::from(r#"{"user_id":99,"message":"bye"}"#);
        let params = build_http_params(&q, &body, true).unwrap().unwrap();
        let obj = params.as_object().unwrap();
        assert_eq!(obj["user_id"], Value::Number(99.into()));
        assert_eq!(obj["message"], Value::String("bye".into()));
    }

    #[test]
    fn http_params_empty_returns_none() {
        let q = HashMap::new();
        let p = build_http_params(&q, &Bytes::new(), false).unwrap();
        assert!(p.is_none());
    }

    #[test]
    fn http_params_skips_access_token() {
        let mut q = HashMap::new();
        q.insert("access_token".into(), "secret".into());
        q.insert("user_id".into(), "1".into());
        let params = build_http_params(&q, &Bytes::new(), false)
            .unwrap()
            .unwrap();
        let obj = params.as_object().unwrap();
        assert!(!obj.contains_key("access_token"));
        assert_eq!(obj["user_id"], Value::Number(1.into()));
    }
}
