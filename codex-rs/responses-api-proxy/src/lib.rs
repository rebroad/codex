use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use anyhow::Result;
use axum::body::Body;
use axum::body::to_bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::HeaderValue;
use axum::http::Method;
use axum::http::Request;
use axum::http::Response;
use axum::http::StatusCode;
use axum::http::Uri;
use axum::response::IntoResponse;
use axum::routing::any;
use axum::routing::get;
use axum::Router;
use bytes::Bytes;
use clap::Parser;
use codex_api::ResponseEvent;
use codex_api::TransportError;
use codex_api::spawn_response_stream;
use codex_login::AuthCredentialsStoreMode;
use codex_client::StreamResponse;
use codex_login::AuthManager;
use codex_state::AccountUsageEstimatorConfig;
use codex_state::AccountUsageEventMeta;
use codex_state::AccountUsageStore;
use codex_utils_home_dir::find_codex_home;
use futures::StreamExt;
use reqwest::Url;
use serde::Deserialize;
use serde_json::Value;
use codex_protocol::protocol::TokenUsage;
use sha2::Digest;
use sha2::Sha256;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tokio::sync::Notify;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

mod dump;
mod read_api_key;

use dump::ExchangeDumper;
use read_api_key::read_auth_header_from_stdin;

const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);
const DEFAULT_PROVIDER_ID: &str = "proxy";
const DEFAULT_ACCOUNT_ID: &str = "proxy";

/// CLI arguments for the proxy.
#[derive(Debug, Clone, Parser)]
#[command(
    name = "responses-api-proxy",
    about = "HTTP proxy for OpenAI/Codex responses traffic",
    version
)]
pub struct Args {
    /// Port to listen on. If not set, an ephemeral port is used.
    #[arg(long)]
    pub port: Option<u16>,

    /// Path to a JSON file to write startup info (single line). Includes {"port": <u16>}.
    #[arg(long, value_name = "FILE")]
    pub server_info: Option<PathBuf>,

    /// Enable HTTP shutdown endpoint at GET /shutdown.
    #[arg(long)]
    pub http_shutdown: bool,

    /// Absolute URL the proxy should forward requests to.
    #[arg(long, default_value = "https://api.openai.com")]
    pub upstream_url: String,

    /// Directory where request/response dumps should be written as JSON.
    #[arg(long, value_name = "DIR")]
    pub dump_dir: Option<PathBuf>,

    /// Override the SQLite home directory used for usage accounting.
    #[arg(long, value_name = "DIR")]
    pub sqlite_home: Option<PathBuf>,

    /// Use the auth token from an auth.json file instead of stdin.
    #[arg(long, value_name = "FILE")]
    pub auth_file: Option<PathBuf>,

    /// Provider ID to use when writing usage rows.
    #[arg(long, default_value = DEFAULT_PROVIDER_ID)]
    pub provider_id: String,

}

#[derive(Clone)]
struct ProxyState {
    client: reqwest::Client,
    upstream_auth_header: Arc<RwLock<String>>,
    auth_file_account_identity: Arc<RwLock<Option<(String, String)>>>,
    upstream_url: Url,
    dump_dir: Option<Arc<ExchangeDumper>>,
    usage_store: Option<Arc<AccountUsageStore>>,
    inflight_requests: Arc<InflightRequestTracker>,
    shutdown_state: Arc<ShutdownState>,
}

struct AuthFileConfig {
    upstream_auth_header: String,
    account_identity: (String, String),
}

struct InflightRequestTracker {
    count: AtomicUsize,
    notify: Notify,
}

impl InflightRequestTracker {
    fn new() -> Self {
        Self {
            count: AtomicUsize::new(0),
            notify: Notify::new(),
        }
    }

    fn start(self: &Arc<Self>) -> InflightRequestGuard {
        self.count.fetch_add(1, Ordering::AcqRel);
        InflightRequestGuard {
            tracker: Arc::clone(self),
        }
    }

    async fn wait_for_zero(&self) {
        loop {
            if self.count.load(Ordering::Acquire) == 0 {
                return;
            }
            self.notify.notified().await;
        }
    }
}

struct InflightRequestGuard {
    tracker: Arc<InflightRequestTracker>,
}

impl Drop for InflightRequestGuard {
    fn drop(&mut self) {
        if self.tracker.count.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.tracker.notify.notify_waiters();
        }
    }
}

struct ShutdownState {
    requested: AtomicBool,
    notify: Notify,
}

impl ShutdownState {
    fn new() -> Self {
        Self {
            requested: AtomicBool::new(false),
            notify: Notify::new(),
        }
    }

    fn request_shutdown(&self) {
        if !self.requested.swap(true, Ordering::AcqRel) {
            self.notify.notify_waiters();
        }
    }

    async fn wait(&self) {
        loop {
            if self.requested.load(Ordering::Acquire) {
                return;
            }
            self.notify.notified().await;
        }
    }
}

#[derive(Deserialize)]
struct JsonResponseCompleted {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    usage: Option<JsonResponseCompletedUsage>,
}

#[derive(Deserialize)]
struct JsonResponseCompletedUsage {
    input_tokens: i64,
    input_tokens_details: Option<JsonResponseCompletedInputTokensDetails>,
    output_tokens: i64,
    output_tokens_details: Option<JsonResponseCompletedOutputTokensDetails>,
    total_tokens: i64,
}

#[derive(Deserialize)]
struct JsonResponseCompletedInputTokensDetails {
    cached_tokens: i64,
}

#[derive(Deserialize)]
struct JsonResponseCompletedOutputTokensDetails {
    reasoning_tokens: i64,
}

/// Entry point for the library main, for parity with other crates.
pub async fn run_main(args: Args) -> Result<()> {
    let upstream_url = Url::parse(&args.upstream_url).context("parsing --upstream-url")?;
    let auth_file_config = if let Some(auth_file) = args.auth_file.as_deref() {
        Some(resolve_auth_file_config(auth_file).await?)
    } else {
        None
    };
    let upstream_auth_header = Arc::new(RwLock::new(match auth_file_config.as_ref() {
        Some(config) => config.upstream_auth_header.clone(),
        None => read_auth_header_from_stdin().map(str::to_string)?,
    }));
    let auth_file_account_identity = Arc::new(RwLock::new(
        auth_file_config.map(|config| config.account_identity),
    ));
    let dump_dir = args
        .dump_dir
        .map(ExchangeDumper::new)
        .transpose()
        .context("creating --dump-dir")?
        .map(Arc::new);
    let sqlite_home = resolve_sqlite_home(args.sqlite_home)?;
    let usage_store = match AccountUsageStore::init_with_estimator_config(
        sqlite_home,
        args.provider_id.clone(),
        AccountUsageEstimatorConfig::default(),
    )
    .await
    {
        Ok(store) => Some(store),
        Err(err) => {
            eprintln!("failed to initialize usage accounting: {err}");
            None
        }
    };
    let state = Arc::new(ProxyState {
        client: reqwest::Client::builder()
            .build()
            .context("building reqwest client")?,
        upstream_auth_header,
        auth_file_account_identity,
        upstream_url,
        dump_dir,
        usage_store,
        inflight_requests: Arc::new(InflightRequestTracker::new()),
        shutdown_state: Arc::new(ShutdownState::new()),
    });

    spawn_auth_reload_signal_handler(Arc::clone(&state), args.auth_file.clone());
    spawn_shutdown_signal_handler(Arc::clone(&state.shutdown_state));

    let listener = bind_listener(args.port).await?;
    if let Some(path) = args.server_info.as_ref() {
        write_server_info(path, listener.local_addr()?.port())?;
    }

    let app_state = Arc::clone(&state);
    let app = if args.http_shutdown {
        Router::new()
            .route("/shutdown", get(shutdown))
            .fallback(any(proxy))
            .with_state(app_state)
    } else {
        Router::new().fallback(any(proxy)).with_state(app_state)
    };

    eprintln!("responses-api-proxy listening on {}", listener.local_addr()?);

    let shutdown_state = Arc::clone(&state.shutdown_state);
    let inflight_requests = Arc::clone(&state.inflight_requests);
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_state.wait().await;
        })
        .await
        .context("serving proxy")?;

    let _ = tokio::time::timeout(
        Duration::from_secs(10),
        inflight_requests.wait_for_zero(),
    )
    .await;

    Ok(())
}

async fn shutdown(State(state): State<Arc<ProxyState>>) -> impl IntoResponse {
    state.shutdown_state.request_shutdown();
    StatusCode::OK
}

async fn proxy(State(state): State<Arc<ProxyState>>, req: Request<Body>) -> Response<Body> {
    if req.method() == Method::CONNECT {
        return proxy_error_response(
            StatusCode::NOT_IMPLEMENTED,
            "CONNECT is not supported by this proxy",
        );
    }

    let incoming_method = req.method().clone();
    let incoming_uri = req.uri().clone();
    let incoming_url = incoming_uri.to_string();
    let (parts, body) = req.into_parts();
    let body_bytes = match to_bytes(body, usize::MAX).await {
        Ok(bytes) => bytes,
        Err(err) => {
            return proxy_error_response(
                StatusCode::BAD_REQUEST,
                &format!("failed to read request body: {err}"),
            );
        }
    };
    let auth_file_account_identity = state.auth_file_account_identity.read().await.clone();
    let (account_id, account_display) =
        resolve_account_identity(auth_file_account_identity.as_ref(), &parts.headers);

    let request_model_slug = extract_request_model_slug(&body_bytes);
    let target_url = match resolve_upstream_url(&state.upstream_url, &incoming_uri) {
        Ok(url) => url,
        Err(err) => {
            return proxy_error_response(
                StatusCode::BAD_REQUEST,
                &format!("failed to resolve upstream URL: {err}"),
            );
        }
    };
    let request_dump = state
        .dump_dir
        .as_ref()
        .and_then(|dumper| dumper.dump_request(&incoming_method, &incoming_url, &parts.headers, &body_bytes).ok());
    let body_bytes = normalize_request_body(body_bytes, &target_url);
    let request_stream = extract_request_stream_flag(&body_bytes);

    let upstream_response = match send_upstream_request(&state, &parts, body_bytes.clone(), target_url.clone()).await {
        Ok(response) => response,
        Err(err) => {
            return proxy_error_response(
                StatusCode::BAD_GATEWAY,
                &format!("failed to forward request upstream: {err}"),
            );
        }
    };

    let status = upstream_response.status();
    let response_headers = sanitize_response_headers(upstream_response.headers());
    let stream_response_headers = sanitize_stream_response_headers(upstream_response.headers());
    let response_content_type = upstream_response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_ascii_lowercase());
    let is_sse = response_content_type
        .as_deref()
        .is_some_and(|value| value.contains("text/event-stream"));
    let is_sse = is_sse
        || (request_stream && target_url.path().ends_with("/responses"));

    if is_sse {
        let (client_tx, client_rx) = mpsc::channel::<Bytes>(16);
        let (parser_tx, parser_rx) = mpsc::channel::<Bytes>(16);
        let response_headers_for_dump = response_headers.clone();
        let stream_response_headers_for_client = stream_response_headers.clone();
        let stream_response_headers_for_task = stream_response_headers;
        let request_dump_path = request_dump;
        let mut response_stream = upstream_response.bytes_stream();
        let status_for_dump = status;
        let inflight_guard = state.inflight_requests.start();
        let token_usage_recorded = Arc::new(AtomicBool::new(false));

        tokio::spawn(async move {
            let usage_task = tokio::spawn(record_usage_events(
                Arc::clone(&state),
                spawn_response_stream(
                    StreamResponse {
                        status,
                        headers: stream_response_headers_for_task.clone(),
                        bytes: ReceiverStream::new(parser_rx)
                            .map(Ok::<Bytes, TransportError>)
                            .boxed(),
                    },
                    STREAM_IDLE_TIMEOUT,
                    None,
                    None,
                    None,
                    body_bytes.len() as i64,
                ),
                state.usage_store.clone(),
                account_id.clone(),
                account_display.clone(),
                request_model_slug.clone(),
                Arc::clone(&token_usage_recorded),
            ));

            let mut response_bytes = Vec::new();
            while let Some(chunk) = response_stream.next().await {
                match chunk {
                    Ok(bytes) => {
                        response_bytes.extend_from_slice(&bytes);
                        let _ = client_tx.send(bytes.clone()).await;
                        let _ = parser_tx.send(bytes).await;
                    }
                    Err(err) => {
                        eprintln!("upstream stream error: {err}");
                        break;
                    }
                }
            }

            if let Some(dumper) = request_dump_path {
                if let Err(err) = dumper.write_response(
                    status_for_dump.as_u16(),
                    &response_headers_for_dump,
                    &response_bytes,
                ) {
                    eprintln!("failed to write response dump: {err}");
                }
            }
            if let Some(usage_store) = state.usage_store.as_ref()
                && !token_usage_recorded.load(Ordering::Acquire)
                && let Some((response_id, token_usage)) =
                    extract_completed_response_usage_from_sse(&response_bytes)
            {
                if token_usage_recorded
                    .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    let model_slug = request_model_slug.as_deref();
                    if let Err(err) = usage_store
                        .record_account_token_usage(
                            &account_id,
                            &token_usage,
                            AccountUsageEventMeta {
                                query_id: response_id.as_deref(),
                                model_slug,
                                sent_bytes: Some(body_bytes.len() as i64),
                                recv_bytes: Some(response_bytes.len() as i64),
                                is_prewarm: false,
                                is_regional_processing: false,
                            },
                        )
                        .await
                    {
                        token_usage_recorded.store(false, Ordering::Release);
                        eprintln!("failed to record fallback usage from SSE response: {err}");
                    }
                }
            }

            let _ = usage_task.await;
            drop(inflight_guard);
        });

        return build_streaming_response(
            status,
            stream_response_headers_for_client,
            client_rx,
        );
    }

    let response_bytes = match upstream_response.bytes().await {
        Ok(bytes) => bytes,
        Err(err) => {
            return proxy_error_response(
                StatusCode::BAD_GATEWAY,
                &format!("failed to read upstream response: {err}"),
            );
        }
    };

    if let Some(dumper) = request_dump
        && let Err(err) = dumper.write_response(
            status.as_u16(),
            &response_headers,
            &response_bytes,
        )
    {
        eprintln!("failed to write response dump: {err}");
    }

    if let Ok(json) = serde_json::from_slice::<JsonResponseCompleted>(&response_bytes)
        && let Some(model_slug) = json_model_slug(&response_headers).or(request_model_slug.as_deref())
    {
        let token_usage = if let Some(usage) = json.usage {
            Some(TokenUsage {
                input_tokens: usage.input_tokens,
                cached_input_tokens: usage
                    .input_tokens_details
                    .map(|details| details.cached_tokens)
                    .unwrap_or(0),
                output_tokens: usage.output_tokens,
                reasoning_output_tokens: usage
                    .output_tokens_details
                    .map(|details| details.reasoning_tokens)
                    .unwrap_or(0),
                total_tokens: usage.total_tokens,
            })
        } else if let Some(response_id) = json.id.as_deref() {
            fetch_completed_response_usage(&state, response_id)
                .await
                .ok()
                .flatten()
        } else {
            None
        };

        if let (Some(usage_store), Some(token_usage)) = (state.usage_store.as_ref(), token_usage) {
            if let Err(err) = usage_store
                .record_account_token_usage(
                    &account_id,
                    &token_usage,
                    AccountUsageEventMeta {
                        query_id: json.id.as_deref(),
                        model_slug: Some(model_slug),
                        sent_bytes: Some(body_bytes.len() as i64),
                        recv_bytes: Some(response_bytes.len() as i64),
                        is_prewarm: false,
                        is_regional_processing: false,
                    },
                )
                .await
            {
                eprintln!("failed to record usage from JSON response: {err}");
            }
        }
    }

    build_buffered_response(status, response_headers, response_bytes)
}

fn build_streaming_response(
    status: StatusCode,
    headers: HeaderMap,
    client_rx: mpsc::Receiver<Bytes>,
) -> Response<Body> {
    let body_stream = ReceiverStream::new(client_rx)
        .map(Ok::<Bytes, Infallible>);
    let mut response = Response::builder().status(status);
    for (name, value) in headers.iter() {
        response = response.header(name, value);
    }
    response
        .body(Body::from_stream(body_stream))
        .unwrap_or_else(|_| Response::new(Body::from("proxy response build failed")))
}

fn spawn_shutdown_signal_handler(state: Arc<ShutdownState>) {
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::signal;
            use tokio::signal::unix::SignalKind;

            let mut sigterm = match signal(SignalKind::terminate()) {
                Ok(signal) => signal,
                Err(err) => {
                    eprintln!("failed to listen for SIGTERM shutdown signal: {err}");
                    return;
                }
            };
            let mut sigint = match signal(SignalKind::interrupt()) {
                Ok(signal) => signal,
                Err(err) => {
                    eprintln!("failed to listen for SIGINT shutdown signal: {err}");
                    return;
                }
            };

            tokio::select! {
                _ = sigterm.recv() => {
                    state.request_shutdown();
                }
                _ = sigint.recv() => {
                    state.request_shutdown();
                }
            }
        }

        #[cfg(not(unix))]
        {
            if let Err(err) = tokio::signal::ctrl_c().await {
                eprintln!("failed to listen for shutdown signal: {err}");
                return;
            }
            state.request_shutdown();
        }
    });
}

fn build_buffered_response(
    status: StatusCode,
    headers: HeaderMap,
    body: Bytes,
) -> Response<Body> {
    let mut response = Response::builder().status(status);
    for (name, value) in headers.iter() {
        response = response.header(name, value);
    }
    response
        .body(Body::from(body))
        .unwrap_or_else(|_| Response::new(Body::from("proxy response build failed")))
}

async fn send_upstream_request(
    state: &ProxyState,
    parts: &axum::http::request::Parts,
    body_bytes: Bytes,
    target_url: Url,
) -> anyhow::Result<reqwest::Response> {
    let mut headers = sanitize_request_headers(&parts.headers);
    let upstream_auth_header = state.upstream_auth_header.read().await.clone();
    let mut auth_header = HeaderValue::from_str(&upstream_auth_header)
        .context("building upstream authorization header")?;
    auth_header.set_sensitive(true);
    headers.insert(axum::http::header::AUTHORIZATION, auth_header);

    let response = state
        .client
        .request(parts.method.clone(), target_url)
        .headers(headers)
        .body(body_bytes)
        .send()
        .await
        .context("sending upstream request")?;
    Ok(response)
}

async fn record_usage_events(
    state: Arc<ProxyState>,
    mut response_stream: codex_api::ResponseStream,
    usage_store: Option<Arc<AccountUsageStore>>,
    account_id: String,
    account_display: String,
    request_model_slug: Option<String>,
    token_usage_recorded: Arc<AtomicBool>,
) {
    if let Some(store) = usage_store.as_ref() {
        store.cache_account_display(&account_id, account_display).await;
    }

    let mut last_server_model: Option<String> = request_model_slug;
    while let Some(event) = response_stream.next().await {
        match event {
            Ok(ResponseEvent::ServerModel(model)) => {
                last_server_model = Some(model);
            }
            Ok(ResponseEvent::RateLimits(snapshot)) => {
                if let Some(store) = usage_store.as_ref()
                    && let Err(err) = store
                        .record_account_backend_rate_limit(&account_id, &snapshot)
                        .await
                {
                    eprintln!("failed to record backend rate limit: {err}");
                }
            }
            Ok(ResponseEvent::Completed {
                token_usage: Some(token_usage),
                capture_id,
                transport_bytes,
                ..
            }) => {
                if let Some(store) = usage_store.as_ref() {
                    let model_slug = last_server_model.as_deref();
                    if token_usage_recorded
                        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                        .is_ok()
                    {
                        if let Err(err) = store
                            .record_account_token_usage(
                                &account_id,
                                &token_usage,
                                AccountUsageEventMeta {
                                    query_id: capture_id.as_deref(),
                                    model_slug,
                                    sent_bytes: transport_bytes.as_ref().map(|value| value.sent),
                                    recv_bytes: transport_bytes.as_ref().map(|value| value.recv),
                                    is_prewarm: false,
                                    is_regional_processing: false,
                                },
                            )
                            .await
                        {
                            token_usage_recorded.store(false, Ordering::Release);
                            eprintln!("failed to record account token usage: {err}");
                        }
                    }
                }
            }
            Ok(ResponseEvent::Completed {
                response_id,
                token_usage: None,
                capture_id,
                transport_bytes,
                ..
            }) => {
                if let Some(store) = usage_store.as_ref() {
                    match fetch_completed_response_usage(&state, &response_id).await {
                        Ok(Some(token_usage)) => {
                            let model_slug = last_server_model.as_deref();
                            if token_usage_recorded
                                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                                .is_ok()
                            {
                                if let Err(err) = store
                                    .record_account_token_usage(
                                        &account_id,
                                        &token_usage,
                                        AccountUsageEventMeta {
                                            query_id: capture_id.as_deref(),
                                            model_slug,
                                            sent_bytes: transport_bytes.as_ref().map(|value| value.sent),
                                            recv_bytes: transport_bytes.as_ref().map(|value| value.recv),
                                            is_prewarm: false,
                                            is_regional_processing: false,
                                        },
                                    )
                                    .await
                                {
                                    token_usage_recorded.store(false, Ordering::Release);
                                    eprintln!("failed to record account token usage: {err}");
                                }
                            }
                        }
                        Ok(None) => {}
                        Err(err) => {
                            eprintln!(
                                "failed to fetch completed response usage for {response_id}: {err}"
                            );
                        }
                    }
                }
            }
            Ok(ResponseEvent::Created)
            | Ok(ResponseEvent::OutputItemAdded(_))
            | Ok(ResponseEvent::OutputItemDone(_))
            | Ok(ResponseEvent::OutputTextDelta(_))
            | Ok(ResponseEvent::ReasoningSummaryDelta { .. })
            | Ok(ResponseEvent::ReasoningContentDelta { .. })
            | Ok(ResponseEvent::ReasoningSummaryPartAdded { .. })
            | Ok(ResponseEvent::ServerReasoningIncluded(_))
            | Ok(ResponseEvent::ModelsEtag(_)) => {}
            Err(err) => {
                eprintln!("failed to observe upstream response stream: {err}");
                return;
            }
        }
    }
}

fn extract_completed_response_usage_from_sse(
    response_bytes: &[u8],
) -> Option<(Option<String>, TokenUsage)> {
    let response_text = std::str::from_utf8(response_bytes).ok()?;
    let response_text = response_text.replace("\r\n", "\n");
    for event_block in response_text.split("\n\n").collect::<Vec<_>>().into_iter().rev() {
        if !event_block.contains("event: response.completed") {
            continue;
        }

        let data = event_block
            .lines()
            .filter_map(|line| {
                line.strip_prefix("data:")
                    .map(str::trim_start)
                    .filter(|value| !value.is_empty())
            })
            .collect::<Vec<_>>()
            .join("\n");
        if data.is_empty() {
            continue;
        }

        let value: Value = serde_json::from_str(&data).ok()?;
        let response = value.get("response")?;
        let response_id = response
            .get("id")
            .and_then(|value| value.as_str())
            .map(str::to_owned);
        let usage = response.get("usage")?;
        let token_usage = token_usage_from_value(usage)?;
        return Some((response_id, token_usage));
    }
    None
}

fn token_usage_from_value(value: &Value) -> Option<TokenUsage> {
    Some(TokenUsage {
        input_tokens: value.get("input_tokens")?.as_i64()?,
        cached_input_tokens: value
            .get("input_tokens_details")
            .and_then(|details| details.get("cached_tokens"))
            .and_then(Value::as_i64)
            .unwrap_or(0),
        output_tokens: value.get("output_tokens")?.as_i64()?,
        reasoning_output_tokens: value
            .get("output_tokens_details")
            .and_then(|details| details.get("reasoning_tokens"))
            .and_then(Value::as_i64)
            .unwrap_or(0),
        total_tokens: value.get("total_tokens")?.as_i64()?,
    })
}

async fn fetch_completed_response_usage(
    state: &ProxyState,
    response_id: &str,
) -> anyhow::Result<Option<codex_protocol::protocol::TokenUsage>> {
    let base = state.upstream_url.as_str().trim_end_matches('/');
    let url = Url::parse(&format!("{base}/responses/{response_id}"))
        .context("building completed response retrieval URL")?;
    let upstream_auth_header = state.upstream_auth_header.read().await.clone();
    let mut auth_header = HeaderValue::from_str(&upstream_auth_header)
        .context("building upstream authorization header")?;
    auth_header.set_sensitive(true);

    let response = state
        .client
        .get(url)
        .header(axum::http::header::AUTHORIZATION, auth_header)
        .send()
        .await
        .context("sending completed response retrieval request")?;

    if !response.status().is_success() {
        return Ok(None);
    }

    let completed = response
        .json::<JsonResponseCompleted>()
        .await
        .context("decoding completed response retrieval payload")?;

    Ok(completed.usage.map(|usage| codex_protocol::protocol::TokenUsage {
        input_tokens: usage.input_tokens,
        cached_input_tokens: usage
            .input_tokens_details
            .map(|details| details.cached_tokens)
            .unwrap_or(0),
        output_tokens: usage.output_tokens,
        reasoning_output_tokens: usage
            .output_tokens_details
            .map(|details| details.reasoning_tokens)
            .unwrap_or(0),
        total_tokens: usage.total_tokens,
    }))
}

fn proxy_error_response(status: StatusCode, message: &str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(axum::http::header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Body::from(message.to_string()))
        .unwrap_or_else(|_| Response::new(Body::from("proxy error")))
}

fn sanitize_request_headers(headers: &HeaderMap) -> HeaderMap {
    let mut forwarded = HeaderMap::new();
    for (name, value) in headers.iter() {
        if is_hop_by_hop_request_header(name) {
            continue;
        }
        forwarded.append(name.clone(), value.clone());
    }
    forwarded
}

fn sanitize_response_headers(headers: &HeaderMap) -> HeaderMap {
    let mut forwarded = HeaderMap::new();
    for (name, value) in headers.iter() {
        if is_hop_by_hop_response_header(name) {
            continue;
        }
        forwarded.append(name.clone(), value.clone());
    }
    forwarded
}

fn sanitize_stream_response_headers(headers: &HeaderMap) -> HeaderMap {
    let mut forwarded = sanitize_response_headers(headers);
    forwarded.remove(axum::http::header::CONTENT_LENGTH);
    forwarded
}

fn is_hop_by_hop_request_header(name: &axum::http::HeaderName) -> bool {
    matches!(
        name.as_str().to_ascii_lowercase().as_str(),
        "connection"
            | "proxy-connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "host"
            | "content-length"
    )
}

fn is_hop_by_hop_response_header(name: &axum::http::HeaderName) -> bool {
    matches!(
        name.as_str().to_ascii_lowercase().as_str(),
        "connection"
            | "proxy-connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn resolve_upstream_url(base_url: &Url, uri: &Uri) -> Result<Url> {
    if uri.scheme().is_some() {
        return Url::parse(&uri.to_string()).context("parse absolute request URI");
    }

    let mut resolved = base_url.clone();
    let combined_path = join_paths(base_url.path(), uri.path());
    resolved.set_path(&combined_path);
    resolved.set_query(uri.query());
    Ok(resolved)
}

fn normalize_request_body(body_bytes: Bytes, target_url: &Url) -> Bytes {
    let Ok(mut value) = serde_json::from_slice::<Value>(&body_bytes) else {
        return body_bytes;
    };

    let Some(object) = value.as_object_mut() else {
        return body_bytes;
    };

    let mut changed = false;
    if object.remove("extra_headers").is_some() {
        changed = true;
    }

    if target_url.path().contains("/backend-api/codex") {
        match object.get("stream") {
            Some(Value::Bool(true)) => {}
            _ => {
                object.insert("stream".to_string(), Value::Bool(true));
                changed = true;
            }
        }
    }

    if changed {
        match serde_json::to_vec(&value) {
            Ok(mut data) => {
                data.shrink_to_fit();
                Bytes::from(data)
            }
            Err(_) => body_bytes,
        }
    } else {
        body_bytes
    }
}

fn join_paths(base_path: &str, request_path: &str) -> String {
    let base = base_path.trim_end_matches('/');
    let request = request_path.trim_start_matches('/');

    match (base.is_empty(), request.is_empty()) {
        (true, true) => "/".to_string(),
        (true, false) => format!("/{request}"),
        (false, true) => format!("{base}/"),
        (false, false) => format!("{base}/{request}"),
    }
}

fn extract_request_model_slug(body_bytes: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(body_bytes).ok()?;
    value
        .get("model")
        .and_then(|value| value.as_str())
        .map(str::to_owned)
}

fn extract_request_stream_flag(body_bytes: &[u8]) -> bool {
    let Ok(value) = serde_json::from_slice::<Value>(body_bytes) else {
        return false;
    };

    value
        .get("stream")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

fn resolve_account_identity(
    auth_file_account_identity: Option<&(String, String)>,
    headers: &HeaderMap,
) -> (String, String) {
    if let Some(identity) = auth_file_account_identity {
        return identity.clone();
    }

    derive_request_account_identity(headers)
}

fn derive_request_account_identity(headers: &HeaderMap) -> (String, String) {
    if let Some(account_id) = headers
        .get("chatgpt-account-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .filter(|value| !value.is_empty())
    {
        return (account_id.clone(), account_id);
    }

    if let Some(authorization) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
    {
        let digest = Sha256::digest(authorization.as_bytes());
        let fingerprint = hex_prefix(&digest, 12);
        let account_id = format!("auth:{fingerprint}");
        return (account_id.clone(), account_id);
    }

    (
        DEFAULT_ACCOUNT_ID.to_string(),
        DEFAULT_ACCOUNT_ID.to_string(),
    )
}

fn derive_auth_file_account_identity(
    account_id: Option<&str>,
    account_email: Option<&str>,
    bearer_token: Option<&str>,
) -> (String, String) {
    if let Some(account_id) = account_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let display = account_email
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| account_id.to_string());
        return (account_id.to_string(), display);
    }

    if let Some(account_email) = account_email
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let account_email = account_email.to_string();
        return (account_email.clone(), account_email);
    }

    if let Some(bearer_token) = bearer_token {
        let digest = Sha256::digest(bearer_token.as_bytes());
        let fingerprint = hex_prefix(&digest, 12);
        let account_id = format!("auth:{fingerprint}");
        return (account_id.clone(), account_id);
    }

    (
        DEFAULT_ACCOUNT_ID.to_string(),
        DEFAULT_ACCOUNT_ID.to_string(),
    )
}

fn hex_prefix(bytes: &[u8], n_bytes: usize) -> String {
    bytes
        .iter()
        .take(n_bytes)
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

fn json_model_slug(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("openai-model")
        .or_else(|| headers.get("x-openai-model"))
        .and_then(|value| value.to_str().ok())
}

fn resolve_sqlite_home(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path);
    }

    if let Ok(raw) = std::env::var(codex_state::SQLITE_HOME_ENV) {
        let raw = raw.trim();
        if !raw.is_empty() {
            let path = PathBuf::from(raw);
            if path.is_absolute() {
                return Ok(path);
            }
            return Ok(std::env::current_dir()
                .context("reading current directory for CODEX_SQLITE_HOME")?
                .join(path));
        }
    }

    find_codex_home().context("resolving default CODEX_HOME")
}

async fn resolve_auth_file_config(auth_file: &std::path::Path) -> Result<AuthFileConfig> {
    let auth_path = auth_file.to_path_buf();
    let codex_home = auth_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(PathBuf::from)
        .or_else(|| find_codex_home().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    codex_login::set_auth_file_override(Some(auth_path));
    let auth_manager = AuthManager::new(
        codex_home,
        /*enable_codex_api_key_env*/ false,
        AuthCredentialsStoreMode::File,
    );
    let auth = auth_manager.auth().await;
    let Some(auth) = auth else {
        return Err(anyhow::anyhow!(
            "failed to load upstream auth from {}",
            auth_file.display()
        ));
    };
    let token = auth
        .get_token()
        .context("reading token from upstream auth file")?;
    let upstream_auth_header = format!("Bearer {token}");
    let account_identity = derive_auth_file_account_identity(
        auth.get_account_id().as_deref(),
        auth.get_account_email().as_deref(),
        Some(&token),
    );
    Ok(AuthFileConfig {
        upstream_auth_header,
        account_identity,
    })
}

fn spawn_auth_reload_signal_handler(state: Arc<ProxyState>, auth_file: Option<PathBuf>) {
    #[cfg(unix)]
    {
        tokio::spawn(async move {
            use tokio::signal::unix::signal;
            use tokio::signal::unix::SignalKind;

            let mut reload = match signal(SignalKind::hangup()) {
                Ok(signal) => signal,
                Err(err) => {
                    eprintln!("failed to listen for SIGHUP reload signal: {err}");
                    return;
                }
            };

            while reload.recv().await.is_some() {
                match auth_file.as_ref() {
                    Some(auth_file) => {
                        match resolve_auth_file_config(auth_file.as_path()).await {
                            Ok(new_config) => {
                                *state.upstream_auth_header.write().await =
                                    new_config.upstream_auth_header;
                                *state.auth_file_account_identity.write().await =
                                    Some(new_config.account_identity);
                                eprintln!(
                                    "received SIGHUP; reloaded upstream auth and accounting identity from {}",
                                    auth_file.display()
                                );
                            }
                            Err(err) => {
                                eprintln!(
                                    "received SIGHUP but failed to reload upstream auth and accounting identity from {}: {err}",
                                    auth_file.display()
                                );
                            }
                        }
                    }
                    None => {
                        eprintln!("received SIGHUP but no --auth-file was configured; ignoring");
                    }
                }
            }
        });
    }

    #[cfg(not(unix))]
    {
        let _ = (state, auth_file);
    }
}

async fn bind_listener(port: Option<u16>) -> Result<TcpListener> {
    let listener = TcpListener::bind(("127.0.0.1", port.unwrap_or(0)))
        .await
        .context("binding proxy listener")?;
    Ok(listener)
}

fn write_server_info(path: &std::path::Path, port: u16) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }

    let payload = serde_json::json!({
        "port": port,
        "pid": std::process::id(),
    });
    let mut data = serde_json::to_vec(&payload)?;
    data.push(b'\n');
    std::fs::write(path, data).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;

    #[test]
    fn account_identity_prefers_chatgpt_account_id_header() {
        let mut headers = HeaderMap::new();
        headers.insert("chatgpt-account-id", HeaderValue::from_static("account-123"));

        let (account_id, account_display) = derive_request_account_identity(&headers);

        assert_eq!(account_id, "account-123");
        assert_eq!(account_display, "account-123");
    }

    #[test]
    fn account_identity_hashes_authorization_when_account_header_missing() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer secret-token"),
        );

        let (account_id, account_display) = derive_request_account_identity(&headers);

        assert!(account_id.starts_with("auth:"));
        assert_eq!(account_display, account_id);
    }

    #[test]
    fn account_identity_falls_back_to_proxy_bucket() {
        let headers = HeaderMap::new();

        let (account_id, account_display) = derive_request_account_identity(&headers);

        assert_eq!(account_id, DEFAULT_ACCOUNT_ID);
        assert_eq!(account_display, DEFAULT_ACCOUNT_ID);
    }

    #[test]
    fn auth_file_account_identity_prefers_account_id_then_email_then_token() {
        let (account_id, account_display) = derive_auth_file_account_identity(
            Some("workspace-123"),
            Some("alice@example.com"),
            Some("unused-token"),
        );
        assert_eq!(account_id, "workspace-123");
        assert_eq!(account_display, "alice@example.com");

        let (account_id, account_display) = derive_auth_file_account_identity(
            None,
            Some("alice@example.com"),
            Some("unused-token"),
        );
        assert_eq!(account_id, "alice@example.com");
        assert_eq!(account_display, "alice@example.com");

        let (account_id, account_display) =
            derive_auth_file_account_identity(None, None, Some("Bearer secret-token"));
        assert!(account_id.starts_with("auth:"));
        assert_eq!(account_display, account_id);
    }

    #[test]
    fn auth_file_identity_overrides_request_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("chatgpt-account-id", HeaderValue::from_static("header-account"));
        let auth_file_identity = Some(("auth-account".to_string(), "auth-display".to_string()));

        let (account_id, account_display) =
            resolve_account_identity(auth_file_identity.as_ref(), &headers);

        assert_eq!(account_id, "auth-account");
        assert_eq!(account_display, "auth-display");
    }

    #[test]
    fn normalize_request_body_strips_extra_headers_and_forces_stream_for_codex_backend() {
        let body = Bytes::from(
            r#"{"model":"gpt-5.4-mini","store":false,"extra_headers":{"session_id":"abc"},"input":[{"role":"user","content":"hi"}]}"#,
        );
        let target_url = Url::parse("https://chatgpt.com/backend-api/codex/responses").unwrap();

        let normalized = normalize_request_body(body, &target_url);
        let value: Value = serde_json::from_slice(&normalized).unwrap();

        assert!(value.get("extra_headers").is_none());
        assert_eq!(value.get("store"), Some(&Value::Bool(false)));
        assert_eq!(value.get("stream"), Some(&Value::Bool(true)));
    }

    #[test]
    fn normalize_request_body_leaves_non_json_unchanged() {
        let body = Bytes::from_static(b"not-json");
        let target_url = Url::parse("https://chatgpt.com/backend-api/codex/responses").unwrap();

        let normalized = normalize_request_body(body.clone(), &target_url);

        assert_eq!(normalized, body);
    }

    #[test]
    fn extract_request_stream_flag_reads_json_bool() {
        let body = Bytes::from_static(br#"{"stream":true}"#);

        assert!(extract_request_stream_flag(&body));
    }

    #[test]
    fn extract_request_stream_flag_defaults_to_false() {
        let body = Bytes::from_static(br#"{"model":"gpt-5.4-mini"}"#);

        assert!(!extract_request_stream_flag(&body));
    }

    #[test]
    fn extract_completed_response_usage_from_sse_parses_usage() {
        let sse = concat!(
            "event: response.created\n",
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\"}}\n\n",
            "event: response.completed\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"usage\":{\"input_tokens\":12,\"input_tokens_details\":{\"cached_tokens\":3},\"output_tokens\":4,\"output_tokens_details\":{\"reasoning_tokens\":1},\"total_tokens\":16}}}\n\n",
        );
        let (response_id, token_usage) =
            extract_completed_response_usage_from_sse(sse.as_bytes()).unwrap();

        assert_eq!(response_id.as_deref(), Some("resp_1"));
        assert_eq!(token_usage.input_tokens, 12);
        assert_eq!(token_usage.cached_input_tokens, 3);
        assert_eq!(token_usage.output_tokens, 4);
        assert_eq!(token_usage.reasoning_output_tokens, 1);
        assert_eq!(token_usage.total_tokens, 16);
    }
}
