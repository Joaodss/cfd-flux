//! The web app served from the server's own origin: a directory (`--web-dir`) or, with the `web`
//! feature, the build embedded in the binary.

mod common;

use cfd_engines::EngineKind;
use cfd_server::Web;
use common::{client, TestServer};
use reqwest::StatusCode;

const INDEX: &str =
    "<!doctype html><title>app</title><script src=\"/assets/app-abc123.js\"></script>";

async fn text(res: reqwest::Response) -> (StatusCode, reqwest::header::HeaderMap, String) {
    let (status, headers) = (res.status(), res.headers().clone());
    (status, headers, res.text().await.unwrap())
}

fn header<'a>(headers: &'a reqwest::header::HeaderMap, name: &str) -> &'a str {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
}

#[tokio::test]
async fn serves_a_directory() {
    let web = std::env::temp_dir().join(format!("cfd-server-web-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&web);
    std::fs::create_dir_all(web.join("assets")).unwrap();
    std::fs::write(web.join("index.html"), INDEX).unwrap();
    std::fs::write(
        web.join("assets/app-abc123.js"),
        "console.log('hi');".repeat(100),
    )
    .unwrap();
    std::fs::write(web.join("secret.txt"), "not served").unwrap();
    // A file next to the web directory, which `..` must not reach.
    let outside = web.with_extension("outside");
    std::fs::write(&outside, "outside").unwrap();

    let dir = web.clone();
    let server = TestServer::with("web-dir", &[EngineKind::Cpu], move |c| {
        c.web = Web::Dir(dir);
    })
    .await;
    let http = client();

    // `/` is index.html, revalidated every time.
    let (status, headers, body) = text(http.get(server.url("/")).send().await.unwrap()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, INDEX);
    assert!(header(&headers, "content-type").starts_with("text/html"));
    assert_eq!(header(&headers, "cache-control"), "no-cache");
    let (status, headers, _) =
        text(http.get(server.url("/index.html")).send().await.unwrap()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header(&headers, "cache-control"), "no-cache");

    // A hashed asset never changes.
    let (status, headers, body) = text(
        http.get(server.url("/assets/app-abc123.js"))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.starts_with("console.log"));
    assert!(header(&headers, "content-type").contains("javascript"));
    assert_eq!(
        header(&headers, "cache-control"),
        "public, max-age=31536000, immutable"
    );

    // Unknown paths: a plain 404 for files, the JSON error under /api.
    let res = http.get(server.url("/nope.js")).send().await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    for path in ["/api/nope", "/api/simulations/x/nope", "/api"] {
        let (status, headers, body) = text(http.get(server.url(path)).send().await.unwrap()).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        assert!(header(&headers, "content-type").contains("json"), "{path}");
        assert!(body.contains("\"error\""), "{path}: {body}");
        assert!(!body.contains("<title>"), "{path}");
        assert!(headers.get("cache-control").is_none(), "{path}");
    }

    // The API is untouched, and carries no cache header of its own.
    let res = http
        .get(server.url("/api/capabilities"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(res.headers().get("cache-control").is_none());

    // Paths cannot leave the directory (the client would normalise a bare `..`, so it is sent
    // percent-encoded, as a hostile client would).
    for path in [
        "/..%2f".to_string() + outside.file_name().unwrap().to_str().unwrap(),
        "/%2e%2e/".to_string() + outside.file_name().unwrap().to_str().unwrap(),
        "/assets/..%2f..%2f".to_string() + outside.file_name().unwrap().to_str().unwrap(),
    ] {
        let (status, _, body) = text(http.get(server.url(&path)).send().await.unwrap()).await;
        assert_ne!(body, "outside", "{path}");
        assert_ne!(status, StatusCode::OK, "{path}");
    }

    // Writing is not allowed.
    let res = http.post(server.url("/index.html")).send().await.unwrap();
    assert_ne!(res.status(), StatusCode::OK);

    server.stop().await;
    let _ = std::fs::remove_dir_all(&web);
    let _ = std::fs::remove_file(&outside);
}

#[tokio::test]
async fn without_a_web_app_everything_but_the_api_is_404() {
    let server = TestServer::start("web-none").await;
    let http = client();
    for path in ["/", "/index.html", "/assets/x.js"] {
        let res = http.get(server.url(path)).send().await.unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "{path}");
    }
    let res = http
        .get(server.url("/api/capabilities"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    server.stop().await;
}

/// With the `web` feature the build in `apps/web/dist` is inside the binary (CI builds it first).
#[cfg(feature = "web")]
#[tokio::test]
async fn serves_the_embedded_app() {
    let server = TestServer::with("web-embedded", &[EngineKind::Cpu], |c| {
        c.web = Web::Embedded;
    })
    .await;
    let http = client();
    let (status, headers, index) = text(http.get(server.url("/")).send().await.unwrap()).await;
    assert_eq!(status, StatusCode::OK);
    assert!(index.contains("<div id=\"root\">"), "{index}");
    assert!(header(&headers, "content-type").starts_with("text/html"));
    assert_eq!(header(&headers, "cache-control"), "no-cache");
    let etag = header(&headers, "etag").to_string();
    assert!(!etag.is_empty());
    let res = http
        .get(server.url("/"))
        .header("if-none-match", &etag)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_MODIFIED);

    // The first script Vite linked from index.html.
    let start = index.find("/assets/").expect("index.html links an asset");
    let end = start + index[start..].find('"').unwrap();
    let asset = &index[start..end];
    let res = http.get(server.url(asset)).send().await.unwrap();
    assert_eq!(res.status(), StatusCode::OK, "{asset}");
    assert_eq!(
        header(res.headers(), "cache-control"),
        "public, max-age=31536000, immutable"
    );
    assert!(!res.bytes().await.unwrap().is_empty());

    let res = http.get(server.url("/api/nope")).send().await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    assert!(header(res.headers(), "content-type").contains("json"));
    server.stop().await;
}
