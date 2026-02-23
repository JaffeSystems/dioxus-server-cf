# dioxus-server-cf

**Patched `dioxus-server` 0.7.3 for `wasm32-unknown-unknown` compatibility.**

This is a minimal fork of [`dioxus-server`](https://crates.io/crates/dioxus-server) that adds `cfg`-gating so the crate compiles for Cloudflare Workers (wasm32 target). It is used by [`dioxus-cloudflare`](https://github.com/JaffeSystems/dioxus-cloudflare) to run Dioxus `#[server]` functions on the Workers runtime.

## Why This Exists

Upstream `dioxus-server` 0.7.3 depends on `tokio` (full), `hyper`, `tower-http`, and other native-only crates that fail to compile on `wasm32-unknown-unknown`. Cloudflare Workers run single-threaded WASM — they don't need a full async runtime, TCP listeners, or HTTP servers.

This fork applies the minimum changes to make compilation succeed:

1. **Gate server-only modules** behind `#[cfg(not(target_arch = "wasm32"))]` in `lib.rs`
2. **Split `server.rs`** into native/wasm submodules — WASM gets a minimal `FullstackState` stub
3. **Split `serverfn.rs` `make_handler`** — native uses `spawn_pinned()`, WASM uses an `AssertSend` wrapper (safe because wasm32 is single-threaded)
4. **Move heavy deps** to `[target.'cfg(not(target_arch = "wasm32"))'.dependencies]`
5. **WASM tokio**: optional, features = `["rt", "sync", "macros"]` only (no `net`)

## Usage

Add this patch to your **workspace** `Cargo.toml`:

```toml
[patch.crates-io]
dioxus-server = { git = "https://github.com/JaffeSystems/dioxus-server-cf.git" }
```

Then depend on `dioxus-server = "=0.7.3"` as normal — Cargo resolves it to this fork.

## What's Changed vs Upstream

| Area | Upstream | This Fork |
|------|----------|-----------|
| Target | Native only | Native + wasm32 |
| Tokio | Full runtime | Minimal (rt, sync, macros) on wasm32 |
| Hyper/tower-http | Always included | `cfg`-gated out on wasm32 |
| `make_handler` | `spawn_pinned()` | `AssertSend` wrapper on wasm32 |
| Server modules | Always compiled | Stubbed on wasm32 |
| API surface | Full | Identical on native; headless-only on wasm32 |

## Maintenance

This fork tracks `dioxus-server` 0.7.3 exactly. When upstream Dioxus releases a new version, this fork will need to be rebased onto the new upstream.

## License

Same as upstream Dioxus — [MIT license](https://github.com/dioxuslabs/dioxus/blob/main/LICENSE-MIT).

Patches and modifications copyright 2026-2027 Jaffe Systems.
