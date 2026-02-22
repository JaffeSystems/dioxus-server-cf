#![doc = include_str!("../README.md")]
#![doc(html_logo_url = "https://avatars.githubusercontent.com/u/79236386")]
#![doc(html_favicon_url = "https://avatars.githubusercontent.com/u/79236386")]
// #![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]
// Patched: allow wasm32 cfg in this fork
#![allow(unexpected_cfgs)]

// re-exported to make it possible to implement a custom Client without adding a separate
// dependency on `bytes`
pub use bytes::Bytes;
pub use dioxus_fullstack_core::{ServerFnError, ServerFnResult};

pub use axum;
pub use http;
pub use inventory;

// --- Server-only modules and re-exports ---
#[cfg(not(target_arch = "wasm32"))]
pub use config::ServeConfig;
#[cfg(not(target_arch = "wasm32"))]
pub use config::*;
#[cfg(not(target_arch = "wasm32"))]
pub use document::ServerDocument;

#[cfg(not(target_arch = "wasm32"))]
pub mod redirect;

#[cfg(not(target_arch = "wasm32"))]
mod launch;

#[cfg(not(target_arch = "wasm32"))]
pub use launch::{launch, launch_cfg};

#[cfg(not(target_arch = "wasm32"))]
pub use launch::router;
#[cfg(not(target_arch = "wasm32"))]
pub use launch::serve;

/// Implementations of the server side of the server function call.
pub mod server;
pub use server::*;

#[cfg(not(target_arch = "wasm32"))]
pub mod config;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod document;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod ssr;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod streaming;

pub mod serverfn;
pub use serverfn::*;

#[cfg(not(target_arch = "wasm32"))]
pub mod isrg;
#[cfg(not(target_arch = "wasm32"))]
pub use isrg::*;

#[cfg(not(target_arch = "wasm32"))]
mod index_html;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use index_html::IndexHtml;
