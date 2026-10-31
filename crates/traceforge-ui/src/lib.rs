//! # TraceForge UI
//!
//! Embedded static web dashboard delivery via `rust-embed` and Axum.

use axum::body::Body;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use tracing::debug;

/// Embedded static assets built from the Next.js export in `dashboard/out`.
#[derive(rust_embed::RustEmbed)]
#[folder = "../../dashboard/out/"]
pub struct DashboardAssets;

/// Determines the MIME Content-Type for a file path, ensuring proper character sets.
pub fn guess_content_type(path: &str) -> &'static str {
    let lower = path.to_lowercase();
    if lower.ends_with(".html") || lower.ends_with(".htm") {
        "text/html; charset=utf-8"
    } else if lower.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if lower.ends_with(".js") || lower.ends_with(".mjs") {
        "application/javascript; charset=utf-8"
    } else if lower.ends_with(".svg") {
        "image/svg+xml"
    } else if lower.ends_with(".json") {
        "application/json; charset=utf-8"
    } else if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".ico") {
        "image/x-icon"
    } else if lower.ends_with(".txt") {
        "text/plain; charset=utf-8"
    } else if lower.ends_with(".woff2") {
        "font/woff2"
    } else if lower.ends_with(".woff") {
        "font/woff"
    } else if let Some(guessed) = mime_guess::from_path(path).first_raw() {
        guessed
    } else {
        "application/octet-stream"
    }
}

/// Helper to build a 200 OK HTTP response with correct headers for an embedded asset.
fn build_asset_response(path: &str, data: std::borrow::Cow<'static, [u8]>) -> Response {
    let content_type = guess_content_type(path);
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, content_type)
        .header(CACHE_CONTROL, "public, max-age=3600")
        .body(Body::from(data))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// Resolves an asset from `DashboardAssets` or falls back to `index.html` for Next.js SPA routing.
pub fn resolve_asset_or_fallback(request_path: &str) -> Response {
    let clean = request_path.trim_start_matches('/');

    // 1. Root and dashboard index requests
    if clean.is_empty() || clean == "dashboard" || clean == "dashboard/" {
        if let Some(index) = DashboardAssets::get("index.html") {
            return build_asset_response("index.html", index.data);
        }
    }

    // 2. Dashboard prefix stripping (e.g. /dashboard/404.html or /dashboard/index.txt)
    if let Some(sub) = clean.strip_prefix("dashboard/") {
        if !sub.is_empty() {
            if let Some(asset) = DashboardAssets::get(sub) {
                return build_asset_response(sub, asset.data);
            }
            let sub_index = format!("{}/index.html", sub.trim_end_matches('/'));
            if let Some(asset) = DashboardAssets::get(&sub_index) {
                return build_asset_response(&sub_index, asset.data);
            }
        }
    }

    // 3. Direct path lookup in embedded assets (including _next/...)
    if let Some(asset) = DashboardAssets::get(clean) {
        return build_asset_response(clean, asset.data);
    }

    // 4. Directory index lookup (e.g. 404/index.html)
    let dir_index = format!("{}/index.html", clean.trim_end_matches('/'));
    if let Some(asset) = DashboardAssets::get(&dir_index) {
        return build_asset_response(&dir_index, asset.data);
    }

    // 5. Next.js SPA Fallback: if not found, return index.html with 200 OK
    debug!("Asset '{}' not found, falling back to index.html for SPA", request_path);
    if let Some(index) = DashboardAssets::get("index.html") {
        build_asset_response("index.html", index.data)
    } else {
        Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header(CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(Body::from("Dashboard index.html not found"))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
    }
}

/// Axum route handler serving embedded static assets from `dashboard/out`.
pub async fn serve_static_asset(uri: Uri) -> Response {
    resolve_asset_or_fallback(uri.path())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Uri;
    use http_body_util::BodyExt;

    #[tokio::test]
    async fn test_serve_dashboard_root() {
        let uri: Uri = "/dashboard".parse().unwrap();
        let resp = serve_static_asset(uri).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            "text/html; charset=utf-8"
        );

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let html = String::from_utf8_lossy(&body);
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("<title>TraceForge"));
    }

    #[tokio::test]
    async fn test_serve_dashboard_slash() {
        let uri: Uri = "/dashboard/".parse().unwrap();
        let resp = serve_static_asset(uri).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            "text/html; charset=utf-8"
        );
    }

    #[tokio::test]
    async fn test_serve_root() {
        let uri: Uri = "/".parse().unwrap();
        let resp = serve_static_asset(uri).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let html = String::from_utf8_lossy(&body);
        assert!(html.contains("<title>TraceForge"));
    }

    #[tokio::test]
    async fn test_serve_next_static_css() {
        let uri: Uri = "/_next/static/css/adb090c985d1da0e.css".parse().unwrap();
        let resp = serve_static_asset(uri).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            "text/css; charset=utf-8"
        );
    }

    #[tokio::test]
    async fn test_serve_spa_fallback_on_unknown_route() {
        let uri: Uri = "/dashboard/sessions/sess_unknown".parse().unwrap();
        let resp = serve_static_asset(uri).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            "text/html; charset=utf-8"
        );
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let html = String::from_utf8_lossy(&body);
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("<title>TraceForge"));
    }

    #[test]
    fn test_mime_type_guessing() {
        assert_eq!(guess_content_type("page.html"), "text/html; charset=utf-8");
        assert_eq!(guess_content_type("style.css"), "text/css; charset=utf-8");
        assert_eq!(
            guess_content_type("app.js"),
            "application/javascript; charset=utf-8"
        );
        assert_eq!(guess_content_type("icon.svg"), "image/svg+xml");
        assert_eq!(guess_content_type("logo.png"), "image/png");
    }
}
