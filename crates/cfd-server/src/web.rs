//! Serving the web app (`apps/web`) from the same origin as the API (ADR-010, design §10).
//!
//! Three situations, chosen by [`Web`] in the [`Config`](crate::Config):
//! - [`Web::None`]: API only (a plain `cargo build`; the Rust CI needs no Node).
//! - [`Web::Dir`]: a directory, served by `tower-http`'s `ServeDir` (development, or a build
//!   without the `web` feature). `ServeDir` does the path handling: `..` cannot leave the directory.
//! - [`Web::Embedded`]: the files of `apps/web/dist`, compiled into the binary by `rust-embed`.
//!   Only with the Cargo feature `web` (in C# terms, a `#if WEB` block: the `dist` folder must
//!   exist at compile time, so `npm run build` comes first, then `cargo build --release --features web`).
//!
//! Caching: Vite names every file under `assets/` after its content hash (the worker included), so
//! those are `immutable`; everything else (`index.html` above all) is `no-cache`, i.e. revalidated
//! on every load, so a new build is picked up at once.

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use axum::extract::Request;
use axum::http::header::CACHE_CONTROL;
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::Router;
use tower_http::services::ServeDir;

use crate::AppState;

/// Where the web app comes from.
#[derive(Debug, Clone, Default)]
pub enum Web {
    /// Only the API.
    #[default]
    None,
    /// Files of a directory (`--web-dir`).
    Dir(PathBuf),
    /// Files compiled into the binary (feature `web`).
    Embedded,
}

impl Web {
    /// Whether the embedded app was compiled in.
    pub const EMBEDDED_AVAILABLE: bool = cfg!(feature = "web");

    /// Adds the static files as the router's fallback: every path no `/api` route took.
    pub fn attach(&self, router: Router<AppState>) -> Router<AppState> {
        match self {
            Web::None => router.fallback(crate::api::not_found),
            Web::Dir(dir) => router
                .fallback_service(ServeDir::new(dir))
                .layer(middleware::from_fn(cache_control)),
            Web::Embedded => router
                .fallback(embedded::serve)
                .layer(middleware::from_fn(cache_control)),
        }
    }
}

/// `Cache-Control` by path. A middleware (like ASP.NET's `app.Use`) that leaves the API alone.
async fn cache_control(req: Request, next: Next) -> Response {
    let path = req.uri().path();
    let api = path == "/api" || path.starts_with("/api/");
    let hashed = path.starts_with("/assets/");
    let mut res = next.run(req).await;
    if !api && (res.status().is_success() || res.status() == StatusCode::NOT_MODIFIED) {
        res.headers_mut().insert(
            CACHE_CONTROL,
            HeaderValue::from_static(if hashed {
                "public, max-age=31536000, immutable"
            } else {
                "no-cache"
            }),
        );
    }
    res
}

#[cfg(feature = "web")]
mod embedded {
    use axum::extract::Request;
    use axum::http::header::{CONTENT_TYPE, ETAG, IF_NONE_MATCH};
    use axum::http::{HeaderValue, Method, StatusCode};
    use axum::response::{IntoResponse, Response};

    /// `#[derive(RustEmbed)]` generates `Dist::get(path)` over the files found in the folder at
    /// compile time (a derive macro that reads the file system, like a C# source generator).
    /// `debug-embed` (in Cargo.toml) embeds in debug builds too, so tests and releases serve the
    /// same bytes. The folder is relative to this crate's `Cargo.toml`.
    #[derive(rust_embed::RustEmbed)]
    #[folder = "../../apps/web/dist/"]
    struct Dist;

    /// The embedded file for a request path (`/` is `index.html`), with its media type and ETag.
    pub async fn serve(req: Request) -> Response {
        if !matches!(*req.method(), Method::GET | Method::HEAD) {
            return StatusCode::METHOD_NOT_ALLOWED.into_response();
        }
        let path = req.uri().path().trim_start_matches('/');
        let path = if path.is_empty() { "index.html" } else { path };
        let Some(file) = Dist::get(path) else {
            return StatusCode::NOT_FOUND.into_response();
        };
        let etag = format!(
            "\"{}\"",
            file.metadata
                .sha256_hash()
                .iter()
                .take(8)
                .fold(String::new(), |s, b| s + &format!("{b:02x}"))
        );
        if req
            .headers()
            .get(IF_NONE_MATCH)
            .is_some_and(|v| v.as_bytes() == etag.as_bytes())
        {
            return (StatusCode::NOT_MODIFIED, [(ETAG, etag)]).into_response();
        }
        let mut res = file.data.into_owned().into_response();
        let headers = res.headers_mut();
        if let Ok(v) = HeaderValue::from_str(&etag) {
            headers.insert(ETAG, v);
        }
        if let Ok(v) = HeaderValue::from_str(file.metadata.mimetype()) {
            headers.insert(CONTENT_TYPE, v);
        }
        res
    }
}

/// Without the feature there is nothing embedded: every path is a 404.
#[cfg(not(feature = "web"))]
mod embedded {
    use axum::http::StatusCode;

    pub async fn serve() -> StatusCode {
        StatusCode::NOT_FOUND
    }
}

/// The address a browser should open for a server listening on `listen`: the unspecified
/// addresses (`0.0.0.0`, `[::]`) mean "every interface" and are not something to browse to, so
/// they become the loopback address.
pub fn browser_url(listen: SocketAddr) -> String {
    let ip = match listen.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::from([127, 0, 0, 1]),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::from([127, 0, 0, 1]),
        ip => ip,
    };
    format!("http://{}/", SocketAddr::new(ip, listen.port()))
}

/// Opens `url` in the default browser. A machine without one (headless, CI) is a warning, not
/// a reason to stop the server.
pub fn open_browser(url: String) {
    // `webbrowser::open` can block (it may wait for the launcher process): off the async threads.
    tokio::task::spawn_blocking(move || match webbrowser::open(&url) {
        Ok(()) => tracing::info!("opened {url} in the browser"),
        Err(e) => tracing::warn!("cannot open a browser ({e}); open {url} yourself"),
    });
}

#[cfg(test)]
mod tests {
    use super::browser_url;

    #[test]
    fn unspecified_addresses_open_loopback() {
        let url = |a: &str| browser_url(a.parse().unwrap());
        assert_eq!(url("0.0.0.0:7878"), "http://127.0.0.1:7878/");
        assert_eq!(url("[::]:9000"), "http://127.0.0.1:9000/");
        assert_eq!(url("127.0.0.1:1"), "http://127.0.0.1:1/");
        assert_eq!(url("[::1]:2"), "http://[::1]:2/");
        assert_eq!(url("192.168.0.5:3"), "http://192.168.0.5:3/");
    }
}
