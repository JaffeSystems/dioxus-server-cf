// ============================================================================
// Native (non-WASM) — full FullstackState with SSR, tokio LocalPool, etc.
// ============================================================================
#[cfg(not(target_arch = "wasm32"))]
mod native {
    use crate::{
        ssr::{SSRError, SsrRendererPool},
        ServeConfig, ServerFunction,
    };
    use axum::{
        body::Body,
        extract::State,
        http::{Request, StatusCode},
        response::{IntoResponse, Response},
        routing::*,
    };
    use dioxus_core::{ComponentFunction, VirtualDom};
    use http::header::*;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use tokio_util::task::LocalPoolHandle;
    use tower::util::MapResponse;
    use tower::ServiceExt;
    use tower_http::services::fs::ServeFileSystemResponseBody;

    /// A extension trait with utilities for integrating Dioxus with your Axum router.
    pub trait DioxusRouterExt {
        fn serve_static_assets(self) -> Router<FullstackState>;
        fn serve_dioxus_application<M: 'static>(
            self,
            cfg: ServeConfig,
            app: impl ComponentFunction<(), M> + Send + Sync,
        ) -> Router<()>;
        #[allow(dead_code)]
        fn register_server_functions(self) -> Router<FullstackState>;
        fn serve_api_application<M: 'static>(
            self,
            cfg: ServeConfig,
            app: impl ComponentFunction<(), M> + Send + Sync,
        ) -> Router<()>
        where
            Self: Sized;
    }

    impl DioxusRouterExt for Router<FullstackState> {
        fn register_server_functions(mut self) -> Router<FullstackState> {
            use std::collections::HashSet;
            let mut seen = HashSet::new();
            for func in ServerFunction::collect() {
                if seen.insert(format!("{} {}", func.method(), func.path())) {
                    tracing::info!("Registering: {} {}", func.method(), func.path());
                    self = self.route(func.path(), func.method_router())
                }
            }
            self
        }

        fn serve_static_assets(self) -> Router<FullstackState> {
            let Some(public_path) = public_path() else {
                return self;
            };
            serve_dir_cached(self, &public_path, &public_path)
        }

        fn serve_api_application<M: 'static>(
            self,
            cfg: ServeConfig,
            app: impl ComponentFunction<(), M> + Send + Sync,
        ) -> Router<()> {
            self.register_server_functions()
                .fallback(get(FullstackState::render_handler))
                .with_state(FullstackState::new(cfg, app))
        }

        fn serve_dioxus_application<M: 'static>(
            self,
            cfg: ServeConfig,
            app: impl ComponentFunction<(), M> + Send + Sync,
        ) -> Router<()> {
            self.register_server_functions()
                .serve_static_assets()
                .fallback(get(FullstackState::render_handler))
                .with_state(FullstackState::new(cfg, app))
        }
    }

    pub async fn render_handler(
        State(state): State<FullstackState>,
        request: Request<Body>,
    ) -> impl IntoResponse {
        FullstackState::render_handler(State(state), request).await
    }

    /// State used by [`FullstackState::render_handler`] to render a dioxus component with axum
    #[derive(Clone)]
    pub struct FullstackState {
        config: ServeConfig,
        build_virtual_dom: Arc<dyn Fn() -> VirtualDom + Send + Sync>,
        renderers: Arc<SsrRendererPool>,
        pub(crate) rt: LocalPoolHandle,
    }

    impl FullstackState {
        pub fn headless() -> Self {
            let rt = LocalPoolHandle::new(
                std::thread::available_parallelism()
                    .map(usize::from)
                    .unwrap_or(1),
            );
            Self {
                renderers: Arc::new(SsrRendererPool::new(4, None)),
                build_virtual_dom: Arc::new(|| {
                    panic!("No root component provided for headless FullstackState")
                }),
                config: ServeConfig::new(),
                rt,
            }
        }

        pub fn new<M: 'static>(
            config: ServeConfig,
            root: impl ComponentFunction<(), M> + Send + Sync + 'static,
        ) -> Self {
            let rt = LocalPoolHandle::new(
                std::thread::available_parallelism()
                    .map(usize::from)
                    .unwrap_or(1),
            );
            Self {
                renderers: Arc::new(SsrRendererPool::new(4, config.incremental.clone())),
                build_virtual_dom: Arc::new(move || VirtualDom::new_with_props(root.clone(), ())),
                config,
                rt,
            }
        }

        pub fn new_with_virtual_dom_factory(
            config: ServeConfig,
            build_virtual_dom: impl Fn() -> VirtualDom + Send + Sync + 'static,
        ) -> Self {
            let rt = LocalPoolHandle::new(
                std::thread::available_parallelism()
                    .map(usize::from)
                    .unwrap_or(1),
            );
            Self {
                renderers: Arc::new(SsrRendererPool::new(4, config.incremental.clone())),
                config,
                build_virtual_dom: Arc::new(build_virtual_dom),
                rt,
            }
        }

        pub fn with_config(mut self, config: ServeConfig) -> Self {
            self.config = config;
            self
        }

        pub async fn render_handler(
            State(state): State<Self>,
            request: Request<Body>,
        ) -> Response {
            let (parts, _) = request.into_parts();
            let response = state
                .renderers
                .clone()
                .render_to(parts, &state.config, &state.rt, {
                    let build_virtual_dom = state.build_virtual_dom.clone();
                    let context_providers = state.config.context_providers.clone();
                    move || {
                        let mut vdom = build_virtual_dom();
                        for state in context_providers.as_slice() {
                            vdom.insert_any_root_context(state());
                        }
                        vdom
                    }
                })
                .await;

            match response {
                Ok((status, headers, freshness, rx)) => {
                    let mut response = Response::builder()
                        .status(status.status)
                        .header(CONTENT_TYPE, "text/html; charset=utf-8")
                        .body(Body::from_stream(rx))
                        .unwrap();
                    freshness.write(response.headers_mut());
                    for (key, value) in headers.into_iter() {
                        if let Some(key) = key {
                            response.headers_mut().insert(key, value);
                        }
                    }
                    response
                }
                Err(SSRError::Incremental(e)) => {
                    tracing::error!("Failed to render page: {}", e);
                    Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(e.to_string())
                        .unwrap()
                        .into_response()
                }
                Err(SSRError::HttpError { status, message }) => Response::builder()
                    .status(status)
                    .body(Body::from(message.unwrap_or_else(|| {
                        status
                            .canonical_reason()
                            .unwrap_or("An unknown error occurred")
                            .to_string()
                    })))
                    .unwrap(),
            }
        }
    }

    pub(crate) fn public_path() -> Option<PathBuf> {
        if let Ok(path) = std::env::var("DIOXUS_PUBLIC_PATH") {
            return Some(PathBuf::from(path));
        }
        Some(
            std::env::current_exe()
                .ok()?
                .parent()
                .unwrap()
                .join("public"),
        )
    }

    fn serve_dir_cached<S>(
        mut router: Router<S>,
        public_path: &Path,
        directory: &Path,
    ) -> Router<S>
    where
        S: Send + Sync + Clone + 'static,
    {
        use tower_http::services::{ServeDir, ServeFile};
        let dir = std::fs::read_dir(directory).unwrap_or_else(|e| {
            panic!(
                "Couldn't read public directory at {:?}: {}",
                &directory, e
            )
        });
        for entry in dir.flatten() {
            let path = entry.path();
            if path == public_path.join("index.html") {
                continue;
            }
            let route = format!(
                "/{}",
                path.strip_prefix(public_path)
                    .unwrap()
                    .iter()
                    .map(|segment| segment.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/")
            );
            if path.is_dir() {
                #[cfg(debug_assertions)]
                {
                    router = router.nest_service(&route, ServeDir::new(&path));
                }
                #[cfg(not(debug_assertions))]
                {
                    router = serve_dir_cached(router, public_path, &path);
                }
            } else {
                let serve_file = ServeFile::new(&path).precompressed_br();
                if file_name_looks_immutable(&route) {
                    router = router.nest_service(&route, cache_response_forever(serve_file))
                } else {
                    router = router.nest_service(&route, serve_file)
                }
            }
        }
        router
    }

    type MappedAxumService<S> = MapResponse<
        S,
        fn(Response<ServeFileSystemResponseBody>) -> Response<ServeFileSystemResponseBody>,
    >;

    fn cache_response_forever<S>(service: S) -> MappedAxumService<S>
    where
        S: ServiceExt<Request<Body>, Response = Response<ServeFileSystemResponseBody>>,
    {
        service.map_response(|mut response: Response<ServeFileSystemResponseBody>| {
            response.headers_mut().insert(
                CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=31536000, immutable"),
            );
            response
        })
    }

    fn file_name_looks_immutable(file_name: &str) -> bool {
        file_name.rsplit_once("-dxh").is_some_and(|(_, hash)| {
            hash.chars()
                .take_while(|c| *c != '.')
                .all(|c| c.is_ascii_hexdigit())
        })
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::*;

// ============================================================================
// WASM — minimal FullstackState stub for Cloudflare Workers / WASM targets.
// No SSR, no tokio LocalPool, no filesystem.
// ============================================================================
#[cfg(target_arch = "wasm32")]
mod wasm {
    /// Minimal state stub for WASM targets. Server functions don't need SSR
    /// or a tokio thread pool — they run directly in the single-threaded
    /// WASM runtime.
    #[derive(Clone)]
    pub struct FullstackState;

    impl FullstackState {
        /// Create a headless state for WASM (no-op).
        pub fn headless() -> Self {
            Self
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use wasm::*;
