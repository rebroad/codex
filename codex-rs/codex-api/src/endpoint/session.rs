use crate::auth::AuthProvider;
use crate::auth::add_auth_headers;
use crate::error::ApiError;
use crate::prompt_debug_http::backend_capture_append_event;
use crate::prompt_debug_http::capture_headers_json;
use crate::provider::Provider;
use crate::telemetry::run_with_request_telemetry;
use codex_client::HttpTransport;
use codex_client::Request;
use codex_client::RequestTelemetry;
use codex_client::Response;
use codex_client::StreamResponse;
use http::HeaderMap;
use http::Method;
use serde_json::Value;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use tracing::instrument;

pub(crate) struct EndpointSession<T: HttpTransport, A: AuthProvider> {
    transport: T,
    provider: Provider,
    auth: A,
    request_telemetry: Option<Arc<dyn RequestTelemetry>>,
}

impl<T: HttpTransport, A: AuthProvider> EndpointSession<T, A> {
    pub(crate) fn new(transport: T, provider: Provider, auth: A) -> Self {
        Self {
            transport,
            provider,
            auth,
            request_telemetry: None,
        }
    }

    pub(crate) fn with_request_telemetry(
        mut self,
        request: Option<Arc<dyn RequestTelemetry>>,
    ) -> Self {
        self.request_telemetry = request;
        self
    }

    pub(crate) fn provider(&self) -> &Provider {
        &self.provider
    }

    fn make_request(
        &self,
        method: &Method,
        path: &str,
        extra_headers: &HeaderMap,
        body: Option<&Value>,
    ) -> Request {
        let mut req = self.provider.build_request(method.clone(), path);
        req.headers.extend(extra_headers.clone());
        if let Some(body) = body {
            req.body = Some(body.clone());
        }
        add_auth_headers(&self.auth, req)
    }

    pub(crate) async fn execute(
        &self,
        method: Method,
        path: &str,
        extra_headers: HeaderMap,
        body: Option<Value>,
    ) -> Result<Response, ApiError> {
        self.execute_with(method, path, extra_headers, body, |_| {})
            .await
    }

    #[instrument(
        name = "endpoint_session.execute_with",
        level = "info",
        skip_all,
        fields(http.method = %method, api.path = path)
    )]
    pub(crate) async fn execute_with<C>(
        &self,
        method: Method,
        path: &str,
        extra_headers: HeaderMap,
        body: Option<Value>,
        configure: C,
    ) -> Result<Response, ApiError>
    where
        C: Fn(&mut Request),
    {
        let make_request = || {
            let mut req = self.make_request(&method, path, &extra_headers, body.as_ref());
            configure(&mut req);
            req
        };
        let attempt_counter = AtomicU64::new(0);
        let path = path.to_string();

        let response = run_with_request_telemetry(
            self.provider.retry.to_policy(),
            self.request_telemetry.clone(),
            make_request,
            |req| {
                let attempt = attempt_counter.fetch_add(1, Ordering::Relaxed) + 1;
                let path_for_request = path.clone();
                let path_for_response = path.clone();
                backend_capture_append_event(serde_json::json!({
                    "kind": "http_request",
                    "transport": "http",
                    "path": path_for_request,
                    "attempt": attempt,
                    "method": req.method.as_str(),
                    "url": req.url,
                    "headers": capture_headers_json(&req.headers),
                    "body": req.body,
                    "compression": format!("{:?}", req.compression),
                    "timeout_ms": req.timeout.map(|timeout| timeout.as_millis()),
                }));
                async move {
                    let result = self.transport.execute(req).await;
                    match &result {
                        Ok(response) => {
                            let body_text = String::from_utf8_lossy(&response.body).to_string();
                            backend_capture_append_event(serde_json::json!({
                                "kind": "http_response",
                                "transport": "http",
                                "path": path_for_response,
                                "attempt": attempt,
                                "status": response.status.as_u16(),
                                "headers": capture_headers_json(&response.headers),
                                "body_bytes": response.body.len(),
                                "body": body_text,
                            }));
                        }
                        Err(err) => {
                            backend_capture_append_event(serde_json::json!({
                                "kind": "http_error",
                                "transport": "http",
                                "path": path_for_response,
                                "attempt": attempt,
                                "error": format!("{err}"),
                            }));
                        }
                    }
                    result
                }
            },
        )
        .await?;

        Ok(response)
    }

    #[instrument(
        name = "endpoint_session.stream_with",
        level = "info",
        skip_all,
        fields(http.method = %method, api.path = path)
    )]
    pub(crate) async fn stream_with<C>(
        &self,
        method: Method,
        path: &str,
        extra_headers: HeaderMap,
        body: Option<Value>,
        configure: C,
    ) -> Result<StreamResponse, ApiError>
    where
        C: Fn(&mut Request),
    {
        let make_request = || {
            let mut req = self.make_request(&method, path, &extra_headers, body.as_ref());
            configure(&mut req);
            req
        };
        let attempt_counter = AtomicU64::new(0);
        let path = path.to_string();

        let stream = run_with_request_telemetry(
            self.provider.retry.to_policy(),
            self.request_telemetry.clone(),
            make_request,
            |req| {
                let attempt = attempt_counter.fetch_add(1, Ordering::Relaxed) + 1;
                let path_for_request = path.clone();
                let path_for_response = path.clone();
                backend_capture_append_event(serde_json::json!({
                    "kind": "http_stream_request",
                    "transport": "http",
                    "path": path_for_request,
                    "attempt": attempt,
                    "method": req.method.as_str(),
                    "url": req.url,
                    "headers": capture_headers_json(&req.headers),
                    "body": req.body,
                    "compression": format!("{:?}", req.compression),
                    "timeout_ms": req.timeout.map(|timeout| timeout.as_millis()),
                }));
                async move {
                    let result = self.transport.stream(req).await;
                    match &result {
                        Ok(response) => {
                            backend_capture_append_event(serde_json::json!({
                                "kind": "http_stream_open",
                                "transport": "http",
                                "path": path_for_response,
                                "attempt": attempt,
                                "status": response.status.as_u16(),
                                "headers": capture_headers_json(&response.headers),
                            }));
                        }
                        Err(err) => {
                            backend_capture_append_event(serde_json::json!({
                                "kind": "http_stream_error",
                                "transport": "http",
                                "path": path_for_response,
                                "attempt": attempt,
                                "error": format!("{err}"),
                            }));
                        }
                    }
                    result
                }
            },
        )
        .await?;

        Ok(stream)
    }
}
