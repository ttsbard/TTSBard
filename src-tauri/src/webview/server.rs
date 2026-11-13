use super::service::{port_in_use_message, START_FAILED_CODE};
use super::upnp::{UpnpFailure, UpnpManager};
use super::{
    templates::{default_css, default_html},
    WebViewSettings, WEBVIEW_BIND_ADDRESS,
};
use crate::events::WebViewSseEvent;
use crate::secret_log::safe_path_for_log;
use crate::webview::security::{is_local_network, validate_token};
use axum::{
    extract::{ConnectInfo, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{sse::Event, IntoResponse, Sse},
    routing::get,
    Router,
};
use futures::{Stream, StreamExt};
use serde::Deserialize;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};

pub type SseSender = broadcast::Sender<WebViewSseEvent>;

const AUTH_COOKIE_NAME: &str = "webview_auth";

// Server state type
#[derive(Clone)]
pub struct ServerState {
    pub sse_tx: SseSender,
    pub templates: TemplateCache,
    pub access_token: Option<String>,
}

#[derive(Clone)]
pub struct TemplateCache {
    html: Arc<RwLock<String>>,
    css: Arc<RwLock<String>>,
    rendered: Arc<RwLock<String>>,
}

impl TemplateCache {
    pub async fn new() -> Result<Self, anyhow::Error> {
        let config_dir = crate::paths::config_root()?.join("webview");

        // Ensure directory exists
        tokio::fs::create_dir_all(&config_dir)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create webview directory: {}", e))?;

        let html_path = config_dir.join("index.html");
        let css_path = config_dir.join("style.css");

        // Create default templates if they don't exist
        if !html_path.exists() {
            tracing::info!(
                html_path = %safe_path_for_log(&html_path),
                "Creating default HTML template"
            );
            tokio::fs::write(&html_path, default_html())
                .await
                .map_err(|e| anyhow::anyhow!("Failed to write default HTML: {}", e))?;
        }

        if !css_path.exists() {
            tracing::info!(
                css_path = %safe_path_for_log(&css_path),
                "Creating default CSS"
            );
            tokio::fs::write(&css_path, default_css())
                .await
                .map_err(|e| anyhow::anyhow!("Failed to write default CSS: {}", e))?;
        }

        let html = tokio::fs::read_to_string(&html_path)
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(
                    error = %e,
                    html_path = %safe_path_for_log(&html_path),
                    "Failed to read HTML, using default"
                );
                default_html()
            });

        let css = tokio::fs::read_to_string(&css_path)
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(
                    error = %e,
                    css_path = %safe_path_for_log(&css_path),
                    "Failed to read CSS, using default"
                );
                default_css()
            });

        let rendered = html.replace("{{CSS}}", &css);

        Ok(Self {
            html: Arc::new(RwLock::new(html)),
            css: Arc::new(RwLock::new(css)),
            rendered: Arc::new(RwLock::new(rendered)),
        })
    }

    pub async fn reload(&self) -> Result<(), anyhow::Error> {
        let config_dir = crate::paths::config_root()?.join("webview");

        let html_path = config_dir.join("index.html");
        let css_path = config_dir.join("style.css");

        let html = tokio::fs::read_to_string(&html_path)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read HTML: {}", e))?;

        let css = tokio::fs::read_to_string(&css_path)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read CSS: {}", e))?;

        let rendered = html.replace("{{CSS}}", &css);

        *self.html.write().await = html;
        *self.css.write().await = css;
        *self.rendered.write().await = rendered;

        Ok(())
    }

    pub async fn get_rendered(&self) -> String {
        self.rendered.read().await.clone()
    }
}

#[derive(Clone)]
pub struct WebViewServer {
    pub settings: Arc<RwLock<WebViewSettings>>,
    pub sse_tx: SseSender,
    pub templates: TemplateCache,
    pub upnp_manager: Option<Arc<UpnpManager>>,
}

impl WebViewServer {
    pub async fn new(settings: Arc<RwLock<WebViewSettings>>) -> Result<Self, anyhow::Error> {
        let templates = TemplateCache::new().await?;
        let s = settings.read().await;
        let port = s.port;
        drop(s);

        // Always create UPnP manager (will be toggled dynamically)
        tracing::info!("Creating UPnP manager for port {}", port);
        let upnp_manager = Some(Arc::new(UpnpManager::new(port)));

        Ok(Self {
            settings,
            sse_tx: broadcast::channel(100).0,
            templates,
            upnp_manager,
        })
    }

    pub async fn start(
        &self,
        readiness: Option<tokio::sync::oneshot::Sender<Result<(), String>>>,
        upnp_error: Option<tokio::sync::mpsc::UnboundedSender<String>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let settings = self.settings.read().await;
        let port = settings.port;
        let addr = format!("{}:{}", WEBVIEW_BIND_ADDRESS, port);

        let access_token = settings.access_token.clone();
        let upnp_enabled = settings.upnp_enabled && access_token.is_some();
        if settings.upnp_enabled && access_token.is_none() {
            tracing::warn!("UPnP disabled because WebView access token is not configured");
        }
        drop(settings);

        let state = ServerState {
            sse_tx: self.sse_tx.clone(),
            templates: self.templates.clone(),
            access_token,
        };

        let app = Router::new()
            .route("/", get(index))
            .route("/auth", get(auth_handler))
            .route("/sse", get(sse_handler))
            .with_state(state);

        let socket_addr: SocketAddr = match addr.parse() {
            Ok(socket_addr) => socket_addr,
            Err(e) => {
                tracing::error!(error = %e, addr = %addr, "WebView server address parse failed");
                let encoded = START_FAILED_CODE.to_string();
                if let Some(readiness) = readiness {
                    let _ = readiness.send(Err(encoded.clone()));
                }
                return Err(encoded.into());
            }
        };

        let listener = match tokio::net::TcpListener::bind(socket_addr).await {
            Ok(listener) => listener,
            Err(e) => {
                // Classify from the OS error kind, never from localized OS text:
                // only an occupied port carries the captured port to the
                // frontend; every other failure is the generic code. The
                // technical error stays in logs.
                let encoded = if e.kind() == std::io::ErrorKind::AddrInUse {
                    port_in_use_message(port)
                } else {
                    START_FAILED_CODE.to_string()
                };
                tracing::error!(error = %e, addr = %addr, "WebView server bind failed");
                if let Some(readiness) = readiness {
                    let _ = readiness.send(Err(encoded.clone()));
                }
                return Err(encoded.into());
            }
        };

        tracing::info!(addr = %addr, "WebView server started");
        if let Some(readiness) = readiness {
            let _ = readiness.send(Ok(()));
        }

        // Optional port forwarding не задерживает readiness TCP listener и не
        // удерживает async lifecycle: router I/O идёт на blocking pool, а его
        // поздний результат гасится epoch-политикой UpnpManager.
        if upnp_enabled {
            if let Some(manager) = self.upnp_manager.clone() {
                let upnp_error = upnp_error.clone();
                tokio::spawn(async move {
                    if let Err(e) = manager.set_enabled(true).await {
                        tracing::warn!(
                            error = %e,
                            "UPnP port forwarding failed, continuing anyway"
                        );
                        if e.failure != UpnpFailure::Superseded && manager.is_desired() {
                            if let Some(tx) = upnp_error {
                                let _ = tx.send(e.failure.code().to_string());
                            }
                        } else {
                            tracing::debug!(
                                failure = ?e.failure,
                                desired = manager.is_desired(),
                                "Suppressing stale UPnP startup error"
                            );
                        }
                    }
                });
            }
        }

        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await?;
        Ok(())
    }

    pub async fn broadcast_text(&self, text: &str) {
        let _ = self.sse_tx.send(WebViewSseEvent::Text(text.to_string()));
    }

    pub async fn broadcast_typing(&self, typing: bool) {
        let _ = self.sse_tx.send(WebViewSseEvent::Typing(typing));
    }

    /// Stop the server and clean up resources (including UPnP)
    ///
    /// Router I/O не блокирует async-поток: закрытие уходит на blocking pool с
    /// ограниченным ожиданием, а поздний mapping гасится epoch-политикой.
    pub async fn stop(&self) {
        tracing::info!("Stopping WebViewServer and cleaning up resources");
        if let Some(manager) = &self.upnp_manager {
            if manager.is_mapping_open() || manager.is_desired() {
                tracing::info!("Removing UPnP port mapping on server stop");
                if let Err(e) = manager.set_enabled(false).await {
                    tracing::warn!(error = %e, "Failed to remove UPnP port mapping on stop");
                }
            }
        }
    }
}

#[derive(Deserialize)]
struct AuthQuery {
    token: Option<String>,
}
/// Maps a `WebViewSseEvent` to an Axum `Event` for SSE serialization.
/// This is the single source of truth for wire format used by both `sse_handler` and tests.
fn to_sse_event(event: &WebViewSseEvent) -> Event {
    match event {
        WebViewSseEvent::Text(text) => {
            let json = serde_json::json!({"text": text}).to_string();
            Event::default().data(json)
        }
        WebViewSseEvent::Typing(typing) => {
            let json = serde_json::json!({"typing": typing}).to_string();
            Event::default().event("typing").data(json)
        }
    }
}

// Helper function to extract cookie from headers
fn get_cookie_from_headers(headers: &HeaderMap, name: &str) -> Option<String> {
    let cookie_header = headers.get("cookie")?.to_str().ok()?;
    cookie_header.split(';').find_map(|pair| {
        let mut parts = pair.trim().splitn(2, '=');
        if parts.next()? == name {
            parts
                .next()
                .map(|s| urlencoding::decode(s).unwrap_or(s.into()).into_owned())
        } else {
            None
        }
    })
}

async fn auth_handler(
    Query(params): Query<AuthQuery>,
    State(state): State<ServerState>,
) -> impl IntoResponse {
    if validate_token(params.token.as_deref(), state.access_token.as_deref()) {
        // Return Set-Cookie header with the token
        let cookie_value = state
            .access_token
            .as_ref()
            .map(|token| {
                format!(
                    "{}={}; HttpOnly; Path=/; SameSite=Lax",
                    AUTH_COOKIE_NAME, token
                )
            })
            .unwrap_or_default();

        let mut response = (StatusCode::OK, "Авторизация успешна").into_response();
        if let Ok(cookie_header) = cookie_value.parse() {
            response
                .headers_mut()
                .insert(header::SET_COOKIE, cookie_header);
        }
        response
    } else {
        (StatusCode::UNAUTHORIZED, "Неверный токен").into_response()
    }
}

async fn sse_handler(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    State(state): State<ServerState>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, StatusCode> {
    // Check authentication
    let is_auth = if is_local_network(addr.ip()) {
        true
    } else {
        let cookie_token = get_cookie_from_headers(&headers, AUTH_COOKIE_NAME);
        validate_token(cookie_token.as_deref(), state.access_token.as_deref())
    };

    if !is_auth {
        tracing::warn!(addr = %addr.ip(), "Unauthorized SSE connection attempt");
        return Err(StatusCode::UNAUTHORIZED);
    }

    let rx = state.sse_tx.subscribe();

    let events = futures::stream::unfold(rx, move |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(sse_event) => return Some((Ok(to_sse_event(&sse_event)), rx)),
                // Медленный клиент отстал от broadcast-буфера. Пропущенные
                // события не воспроизводятся, но соединение остаётся живым:
                // клиент продолжает получать последующие события.
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(
                        skipped,
                        "SSE client lagged behind the broadcast buffer, skipping missed events"
                    );
                    continue;
                }
                // Отправитель закрыт: поток больше не получит событий.
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });

    // Emit an immediate data event so clients can distinguish a live SSE
    // handshake from an idle stream that has not connected yet. A data event
    // is used instead of a comment because some SSE clients reject empty
    // comment frames as invalid events.
    let stream =
        futures::stream::once(async { Ok(Event::default().event("connected").data("{}")) })
            .chain(events);

    Ok(Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new().interval(std::time::Duration::from_secs(10)),
    ))
}

async fn index(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Query(params): Query<AuthQuery>,
    headers: HeaderMap,
    State(state): State<ServerState>,
) -> impl IntoResponse {
    // Check authentication
    let is_auth = if is_local_network(addr.ip()) {
        true
    } else {
        // First check cookie
        let cookie_token = get_cookie_from_headers(&headers, AUTH_COOKIE_NAME);
        let cookie_valid = validate_token(cookie_token.as_deref(), state.access_token.as_deref());

        // If cookie invalid, check query parameter token
        if cookie_valid {
            true
        } else {
            validate_token(params.token.as_deref(), state.access_token.as_deref())
        }
    };

    if !is_auth {
        tracing::warn!(addr = %addr.ip(), "Unauthorized page access attempt");
        return StatusCode::UNAUTHORIZED.into_response();
    }

    // If token provided via query and is valid, set cookie for future requests
    let response = if params.token.is_some()
        && validate_token(params.token.as_deref(), state.access_token.as_deref())
    {
        let cookie_value = state
            .access_token
            .as_ref()
            .map(|token| {
                format!(
                    "{}={}; HttpOnly; Path=/; SameSite=Lax",
                    AUTH_COOKIE_NAME, token
                )
            })
            .unwrap_or_default();

        let mut resp = (
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            state.templates.get_rendered().await,
        )
            .into_response();
        if let Ok(cookie_header) = cookie_value.parse() {
            resp.headers_mut().insert(header::SET_COOKIE, cookie_header);
        }
        resp
    } else {
        (
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            state.templates.get_rendered().await,
        )
            .into_response()
    };

    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::webview::upnp::{RouterPortMapper, UpnpError, UpnpFailure};
    use axum::{
        body::{to_bytes, Body},
        http::{Request, StatusCode},
        routing::get,
        Router,
    };
    use std::convert::Infallible;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use tower::ServiceExt;

    fn test_state(token: Option<String>, sse_capacity: usize) -> ServerState {
        ServerState {
            sse_tx: broadcast::channel(sse_capacity).0,
            templates: TemplateCache {
                html: Arc::new(RwLock::new("<html>{{CSS}}</html>".to_string())),
                css: Arc::new(RwLock::new("body {}".to_string())),
                rendered: Arc::new(RwLock::new("<html>body {}</html>".to_string())),
            },
            access_token: token,
        }
    }

    fn build_test_app(token: Option<String>) -> Router {
        Router::new()
            .route("/", get(index))
            .route("/auth", get(auth_handler))
            .route("/sse", get(sse_handler))
            .with_state(test_state(token, 10))
    }

    /// Router с настраиваемой ёмкостью broadcast-буфера и отправителем, чтобы
    /// тест мог переполнить буфер и наблюдать поведение потока.
    fn build_test_app_with_sse(token: Option<String>, sse_capacity: usize) -> (Router, SseSender) {
        let state = test_state(token, sse_capacity);
        let sender = state.sse_tx.clone();
        let app = Router::new()
            .route("/", get(index))
            .route("/auth", get(auth_handler))
            .route("/sse", get(sse_handler))
            .with_state(state);
        (app, sender)
    }

    async fn next_sse_chunk(body: &mut axum::body::BodyDataStream) -> String {
        let chunk = tokio::time::timeout(std::time::Duration::from_secs(2), body.next())
            .await
            .expect("SSE chunk timed out")
            .expect("SSE stream ended early")
            .expect("SSE chunk failed");
        String::from_utf8(chunk.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn test_sse_stream_continues_after_lag_and_ends_when_sender_closes() {
        // Ёмкость 1: три отправки без чтения гарантируют RecvError::Lagged.
        let (app, sse_tx) = build_test_app_with_sse(Some("secret-token".to_string()), 1);

        let client_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 12345);
        let req = Request::builder()
            .uri("/sse")
            .extension(ConnectInfo(client_addr))
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let mut body = response.into_body().into_data_stream();

        let first = next_sse_chunk(&mut body).await;
        assert!(
            first.contains("event: connected"),
            "first chunk must confirm the connection: {first:?}"
        );

        for text in ["first", "second", "third"] {
            sse_tx
                .send(WebViewSseEvent::Text(text.to_string()))
                .expect("SSE broadcast must have a receiver");
        }

        // Lag не завершает поток: клиент получает следующее доступное событие.
        let after_lag = next_sse_chunk(&mut body).await;
        assert!(
            after_lag.contains("data:"),
            "stream must survive a lag: {after_lag:?}"
        );

        // Закрытие отправителя — единственная причина завершить поток.
        drop(sse_tx);
        let end = tokio::time::timeout(std::time::Duration::from_secs(2), body.next()).await;
        assert!(
            matches!(end, Ok(None)),
            "closed sender must end the stream: {end:?}"
        );
    }

    #[tokio::test]
    async fn test_to_sse_event_text_produces_exact_wire_format() {
        let sse_event = WebViewSseEvent::Text("hello".to_string());
        let event = to_sse_event(&sse_event);
        let sse = Sse::new(futures::stream::once(
            async move { Ok::<_, Infallible>(event) },
        ));
        let body = to_bytes(sse.into_response().into_body(), usize::MAX)
            .await
            .unwrap();
        let wire = String::from_utf8(body.to_vec()).unwrap();
        assert_eq!(
            wire, "data: {\"text\":\"hello\"}\n\n",
            "text SSE must be unnamed with exact Axum spacing"
        );
    }

    #[tokio::test]
    async fn test_to_sse_event_typing_true_produces_exact_wire_format() {
        let sse_event = WebViewSseEvent::Typing(true);
        let event = to_sse_event(&sse_event);
        let sse = Sse::new(futures::stream::once(
            async move { Ok::<_, Infallible>(event) },
        ));
        let body = to_bytes(sse.into_response().into_body(), usize::MAX)
            .await
            .unwrap();
        let wire = String::from_utf8(body.to_vec()).unwrap();
        assert_eq!(
            wire, "event: typing\ndata: {\"typing\":true}\n\n",
            "typing true SSE must be named with exact Axum spacing"
        );
    }

    #[tokio::test]
    async fn test_to_sse_event_typing_false_produces_exact_wire_format() {
        let sse_event = WebViewSseEvent::Typing(false);
        let event = to_sse_event(&sse_event);
        let sse = Sse::new(futures::stream::once(
            async move { Ok::<_, Infallible>(event) },
        ));
        let body = to_bytes(sse.into_response().into_body(), usize::MAX)
            .await
            .unwrap();
        let wire = String::from_utf8(body.to_vec()).unwrap();
        assert_eq!(
            wire, "event: typing\ndata: {\"typing\":false}\n\n",
            "typing false SSE must be named with exact Axum spacing"
        );
    }

    #[tokio::test]
    async fn test_access_from_loopback_allowed_without_token() {
        let app = build_test_app(Some("secret-token".to_string()));

        let client_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 12345);
        let req = Request::builder()
            .uri("/")
            .extension(ConnectInfo(client_addr))
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_access_from_private_network_allowed_without_token() {
        let app = build_test_app(Some("secret-token".to_string()));

        let client_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)), 12345);
        let req = Request::builder()
            .uri("/")
            .extension(ConnectInfo(client_addr))
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_access_from_public_network_denied_without_token() {
        let app = build_test_app(Some("secret-token".to_string()));

        let client_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)), 12345);
        let req = Request::builder()
            .uri("/")
            .extension(ConnectInfo(client_addr))
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_access_from_public_network_denied_when_server_token_is_missing() {
        let app = build_test_app(None);

        let client_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)), 12345);
        let req = Request::builder()
            .uri("/")
            .extension(ConnectInfo(client_addr))
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_sse_from_public_network_denied_when_server_token_is_missing() {
        let app = build_test_app(None);

        let client_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)), 12345);
        let req = Request::builder()
            .uri("/sse")
            .extension(ConnectInfo(client_addr))
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_auth_denied_when_server_token_is_missing() {
        let app = build_test_app(None);
        let req = Request::builder().uri("/auth").body(Body::empty()).unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_access_from_public_network_allowed_with_query_token() {
        let app = build_test_app(Some("secret-token".to_string()));

        let client_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)), 12345);
        let req = Request::builder()
            .uri("/?token=secret-token")
            .extension(ConnectInfo(client_addr))
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let cookie = response.headers().get("set-cookie").unwrap();
        assert!(cookie
            .to_str()
            .unwrap()
            .contains("webview_auth=secret-token"));
    }

    #[tokio::test]
    async fn test_access_from_public_network_allowed_with_cookie_token() {
        let app = build_test_app(Some("secret-token".to_string()));

        let client_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)), 12345);
        let req = Request::builder()
            .uri("/")
            .header("cookie", "webview_auth=secret-token")
            .extension(ConnectInfo(client_addr))
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_access_from_public_network_denied_with_wrong_token() {
        let app = build_test_app(Some("secret-token".to_string()));

        let client_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)), 12345);
        let req = Request::builder()
            .uri("/?token=wrong-token")
            .extension(ConnectInfo(client_addr))
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    fn build_server_with_upnp(
        mapper: Arc<dyn crate::webview::upnp::RouterPortMapper>,
        port: u16,
    ) -> WebViewServer {
        build_server_with_upnp_and_timeouts(
            mapper,
            port,
            crate::webview::upnp::UPNP_FORWARD_TIMEOUT,
        )
    }

    fn build_server_with_upnp_and_timeouts(
        mapper: Arc<dyn crate::webview::upnp::RouterPortMapper>,
        port: u16,
        forward_timeout: std::time::Duration,
    ) -> WebViewServer {
        WebViewServer {
            settings: Arc::new(RwLock::new(WebViewSettings {
                port,
                access_token: Some("secret-token".to_string()),
                upnp_enabled: true,
                ..WebViewSettings::default()
            })),
            sse_tx: broadcast::channel(10).0,
            templates: TemplateCache {
                html: Arc::new(RwLock::new("<html>{{CSS}}</html>".to_string())),
                css: Arc::new(RwLock::new("body {}".to_string())),
                rendered: Arc::new(RwLock::new("<html>body {}</html>".to_string())),
            },
            upnp_manager: Some(Arc::new(UpnpManager::with_mapper_and_timeouts(
                port,
                mapper,
                forward_timeout,
                crate::webview::upnp::UPNP_REMOVE_TIMEOUT,
            ))),
        }
    }

    #[tokio::test]
    async fn occupied_port_readiness_reports_captured_port_without_os_text() {
        use crate::webview::upnp::test_support::FakeRouterPortMapper;

        let occupied = tokio::net::TcpListener::bind("0.0.0.0:0").await.unwrap();
        let port = occupied.local_addr().unwrap().port();
        let (mapper, _started, _release) = FakeRouterPortMapper::gated_open();
        let server = build_server_with_upnp(Arc::new(mapper), port);
        server.settings.write().await.upnp_enabled = false;
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();

        let error = server.start(Some(ready_tx), None).await.unwrap_err();
        assert_eq!(error.to_string(), format!("port_in_use:{port}"));
        assert_eq!(ready_rx.await.unwrap(), Err(format!("port_in_use:{port}")));
        drop(occupied);
    }

    #[tokio::test]
    async fn test_readiness_does_not_wait_for_slow_upnp_and_stop_removes_the_mapping() {
        use crate::webview::upnp::test_support::{FakeRouterPortMapper, RouterCall};
        use std::time::Duration;

        let (mapper, mut started, release) = FakeRouterPortMapper::gated_open();
        let mapper = Arc::new(mapper);
        let mapping: Arc<dyn crate::webview::upnp::RouterPortMapper> = mapper.clone();
        // Порт 0: listener получает свободный порт, readiness не зависит от router.
        let server = build_server_with_upnp(mapping, 0);

        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let serve = {
            let server = server.clone();
            tokio::spawn(async move {
                if let Err(e) = server.start(Some(ready_tx), None).await {
                    tracing::warn!(error = %e, "test server start failed");
                }
            })
        };

        // Readiness приходит раньше, чем router ответил на открытие mapping.
        let ready = tokio::time::timeout(Duration::from_secs(5), ready_rx)
            .await
            .expect("readiness must arrive")
            .expect("readiness channel must stay open");
        assert!(ready.is_ok(), "listener must bind: {ready:?}");

        let call = tokio::time::timeout(Duration::from_secs(2), started.recv())
            .await
            .expect("forwarding must start after readiness")
            .expect("started channel must stay open");
        assert_eq!(call, RouterCall::Open(0));

        let manager = server.upnp_manager.clone().expect("manager is configured");
        assert!(
            !manager.is_mapping_open(),
            "forwarding is still pending in the router"
        );

        // Открытие завершается: mapping подтверждается и снимается на stop.
        release.send(()).unwrap();
        for _ in 0..200 {
            if manager.is_mapping_open() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(
            manager.is_mapping_open(),
            "completed open must be confirmed"
        );

        server.stop().await;

        assert!(!manager.is_mapping_open());
        assert!(
            mapper.close_count() >= 1,
            "stop must remove the mapping: {:?}",
            mapper.calls()
        );

        serve.abort();
    }

    #[tokio::test]
    async fn test_superseded_startup_forward_does_not_emit_error_code() {
        use crate::webview::upnp::test_support::{FakeRouterPortMapper, RouterCall};
        use std::time::Duration;

        let (mapper, mut started, release) = FakeRouterPortMapper::gated_open();
        let mapper = Arc::new(mapper);
        let mapping: Arc<dyn crate::webview::upnp::RouterPortMapper> = mapper.clone();
        let server = build_server_with_upnp(mapping, 0);

        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let (upnp_error_tx, mut upnp_error_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let serve = {
            let server = server.clone();
            tokio::spawn(async move {
                if let Err(e) = server.start(Some(ready_tx), Some(upnp_error_tx)).await {
                    tracing::warn!(error = %e, "test server start failed");
                }
            })
        };

        // Readiness приходит раньше, чем router ответил на открытие mapping.
        let ready = tokio::time::timeout(Duration::from_secs(5), ready_rx)
            .await
            .expect("readiness must arrive")
            .expect("readiness channel must stay open");
        assert!(ready.is_ok(), "listener must bind: {ready:?}");

        // Открытие началось, но всё ещё висит в router.
        let call = tokio::time::timeout(Duration::from_secs(2), started.recv())
            .await
            .expect("forwarding must start after readiness")
            .expect("started channel must stay open");
        assert_eq!(call, RouterCall::Open(0));

        // Намерение снимается, пока открытие в полёте.
        let manager = server.upnp_manager.clone().expect("manager is configured");
        manager.set_enabled(false).await.unwrap();

        // Поздний успех открытия гасится как Superseded и не должен уйти наружу.
        release.send(()).unwrap();

        let emitted = tokio::time::timeout(Duration::from_secs(2), upnp_error_rx.recv()).await;
        assert!(
            emitted.is_err(),
            "superseded forward must not emit a startup error code: {emitted:?}"
        );

        serve.abort();
    }

    #[tokio::test]
    async fn test_desired_timeout_still_emits_error_code() {
        use crate::webview::upnp::test_support::FakeRouterPortMapper;
        use std::time::Duration;

        let (mapper, _started, _release) = FakeRouterPortMapper::gated_open();
        let mapper = Arc::new(mapper);
        let mapping: Arc<dyn crate::webview::upnp::RouterPortMapper> = mapper.clone();
        let server = build_server_with_upnp_and_timeouts(mapping, 0, Duration::from_millis(100));

        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let (upnp_error_tx, mut upnp_error_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let serve = {
            let server = server.clone();
            tokio::spawn(async move {
                if let Err(e) = server.start(Some(ready_tx), Some(upnp_error_tx)).await {
                    tracing::warn!(error = %e, "test server start failed");
                }
            })
        };

        let ready = tokio::time::timeout(Duration::from_secs(5), ready_rx)
            .await
            .expect("readiness must arrive")
            .expect("readiness channel must stay open");
        assert!(ready.is_ok(), "listener must bind: {ready:?}");

        // Намерение всё ещё желаемое: timeout — это реальный отказ, и его код
        // должен дойти до фронтенда.
        let code = tokio::time::timeout(Duration::from_secs(5), upnp_error_rx.recv())
            .await
            .expect("UPnP error code must arrive")
            .expect("UPnP error sender must stay open");
        assert_eq!(code, "webview.upnp.timeout");

        serve.abort();
    }

    struct RejectingMapper;

    impl RouterPortMapper for RejectingMapper {
        fn open(&self, _port: u16) -> Result<(), UpnpError> {
            Err(UpnpError::new(
                UpnpFailure::RouterRejected,
                "router refused",
            ))
        }

        fn close(&self, _port: u16) -> Result<(), UpnpError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_failed_upnp_startup_emits_error_code_while_listener_stays_alive() {
        use std::time::Duration;

        let mapping: Arc<dyn crate::webview::upnp::RouterPortMapper> = Arc::new(RejectingMapper);
        let server = build_server_with_upnp(mapping, 0);

        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let (upnp_error_tx, mut upnp_error_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let serve = {
            let server = server.clone();
            tokio::spawn(async move {
                if let Err(e) = server.start(Some(ready_tx), Some(upnp_error_tx)).await {
                    tracing::warn!(error = %e, "test server start failed");
                }
            })
        };

        // Readiness приходит независимо от отказавшего router-вызова.
        let ready = tokio::time::timeout(Duration::from_secs(5), ready_rx)
            .await
            .expect("readiness must arrive")
            .expect("readiness channel must stay open");
        assert!(ready.is_ok(), "listener must bind: {ready:?}");

        // Фоновый UPnP-проброс отвалился: наружу уходит только код отказа.
        let code = tokio::time::timeout(Duration::from_secs(5), upnp_error_rx.recv())
            .await
            .expect("UPnP error code must arrive")
            .expect("UPnP error sender must stay open");
        assert_eq!(code, "webview.upnp.router_rejected");

        // Слушатель остаётся живым после провалившегося UPnP-проброса.
        assert!(!serve.is_finished(), "listener must keep running");

        serve.abort();
    }
}
