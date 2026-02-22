use crate::FullstackState;
use axum::{
    extract::{Request, State},
    response::Response,
    routing::MethodRouter,
};
use dioxus_fullstack_core::FullstackContext;
use http::Method;
use std::{pin::Pin, prelude::rust_2024::Future};

/// A function endpoint that can be called from the client.
#[derive(Clone)]
pub struct ServerFunction {
    path: &'static str,
    method: Method,
    handler: fn() -> MethodRouter<FullstackState>,
}

impl ServerFunction {
    /// Create a new server function object from a MethodRouter
    pub const fn new(
        method: Method,
        path: &'static str,
        handler: fn() -> MethodRouter<FullstackState>,
    ) -> Self {
        Self {
            path,
            method,
            handler,
        }
    }

    /// The path of the server function.
    pub fn path(&self) -> &'static str {
        self.path
    }

    /// The HTTP method the server function expects.
    pub fn method(&self) -> Method {
        self.method.clone()
    }

    /// Collect all globally registered server functions
    pub fn collect() -> Vec<&'static ServerFunction> {
        inventory::iter::<ServerFunction>().collect()
    }

    /// Create a `MethodRouter` for this server function that can be mounted on an `axum::Router`.
    pub fn method_router(&self) -> MethodRouter<FullstackState> {
        (self.handler)()
    }

    /// Creates a new `MethodRouter` for the given method and handler.
    ///
    /// On native targets, this runs the handler inside a tokio `LocalPool`
    /// to support !Send futures. On WASM targets (Cloudflare Workers),
    /// it runs directly in the single-threaded runtime.
    #[allow(clippy::type_complexity)]
    pub fn make_handler(
        method: Method,
        handler: fn(State<FullstackContext>, Request) -> Pin<Box<dyn Future<Output = Response>>>,
    ) -> MethodRouter<FullstackState> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            make_handler_native(method, handler)
        }
        #[cfg(target_arch = "wasm32")]
        {
            make_handler_wasm(method, handler)
        }
    }
}

/// Native: run handler inside tokio LocalPool for !Send future support.
#[cfg(not(target_arch = "wasm32"))]
fn make_handler_native(
    method: Method,
    handler: fn(State<FullstackContext>, Request) -> Pin<Box<dyn Future<Output = Response>>>,
) -> MethodRouter<FullstackState> {
    use axum::body::Body;
    use http::StatusCode;

    axum::routing::method_routing::on(
        method
            .try_into()
            .expect("MethodFilter only supports standard HTTP methods"),
        move |state: State<FullstackState>, request: Request| async move {
            use tracing::Instrument;
            let current_span = tracing::Span::current();
            let result = state.rt.spawn_pinned(move || async move {
                use http::header::{ACCEPT, LOCATION, REFERER};

                let (parts, body) = request.into_parts();
                let server_context = FullstackContext::new(parts.clone());
                let request = axum::extract::Request::from_parts(parts, body);

                let referrer = request.headers().get(REFERER).cloned();
                let accepts_html = request
                    .headers()
                    .get(ACCEPT)
                    .and_then(|v| v.to_str().ok())
                    .map(|v| v.contains("text/html"))
                    .unwrap_or(false);

                server_context
                    .clone()
                    .scope(async move {
                        let mut response = handler(State(server_context), request)
                            .instrument(current_span)
                            .await;

                        let server_context = FullstackContext::current().expect(
                            "Server context should be available inside the server context scope",
                        );

                        let headers = server_context.take_response_headers();
                        if let Some(headers) = headers {
                            response.headers_mut().extend(headers);
                        }

                        if accepts_html {
                            if let Some(referrer) = referrer {
                                let has_location = response.headers().get(LOCATION).is_some();
                                if !has_location {
                                    *response.status_mut() = StatusCode::FOUND;
                                    response.headers_mut().insert(LOCATION, referrer);
                                }
                            }
                        }

                        response
                    })
                    .await
            })
            .await;

            match result {
                Ok(response) => response,
                Err(err) => Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Body::new(if cfg!(debug_assertions) {
                        format!("Server function panicked: {}", err)
                    } else {
                        "Internal Server Error".to_string()
                    }))
                    .unwrap(),
            }
        },
    )
}

/// A wrapper that asserts a future is `Send`.
///
/// On `wasm32-unknown-unknown`, there is only one thread — data races are
/// impossible — so every future is trivially `Send`. Axum's `Handler` trait
/// still requires the bound, though, which the `#[server]` macro's generated
/// `Pin<Box<dyn Future<Output = Response>>>` does not satisfy.
#[cfg(target_arch = "wasm32")]
struct AssertSend<F>(F);

// SAFETY: wasm32-unknown-unknown is single-threaded. There are no other
// threads to send the future to, so the `Send` bound is vacuously true.
#[cfg(target_arch = "wasm32")]
unsafe impl<F> Send for AssertSend<F> {}

#[cfg(target_arch = "wasm32")]
impl<F: Future> Future for AssertSend<F> {
    type Output = F::Output;

    fn poll(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        // SAFETY: We are not moving `F`, only projecting through the pin.
        unsafe { self.map_unchecked_mut(|s| &mut s.0).poll(cx) }
    }
}

/// WASM: run handler directly — no thread pool needed.
/// Workers are single-threaded per isolate.
#[cfg(target_arch = "wasm32")]
fn make_handler_wasm(
    method: Method,
    handler: fn(State<FullstackContext>, Request) -> Pin<Box<dyn Future<Output = Response>>>,
) -> MethodRouter<FullstackState> {
    axum::routing::method_routing::on(
        method
            .try_into()
            .expect("MethodFilter only supports standard HTTP methods"),
        move |request: Request| {
            AssertSend(async move {
                let (parts, body) = request.into_parts();
                let server_context = FullstackContext::new(parts.clone());
                let request = axum::extract::Request::from_parts(parts, body);

                server_context
                    .clone()
                    .scope(async move {
                        let mut response =
                            handler(State(server_context.clone()), request).await;

                        // Apply any response headers set during the handler
                        if let Some(ctx) = FullstackContext::current() {
                            let headers = ctx.take_response_headers();
                            if let Some(headers) = headers {
                                response.headers_mut().extend(headers);
                            }
                        }

                        response
                    })
                    .await
            })
        },
    )
}

impl inventory::Collect for ServerFunction {
    #[inline]
    fn registry() -> &'static inventory::Registry {
        static REGISTRY: inventory::Registry = inventory::Registry::new();
        &REGISTRY
    }
}
