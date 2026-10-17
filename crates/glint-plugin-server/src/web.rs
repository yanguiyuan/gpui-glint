//! Embedded public catalogue; static resources never interpolate plugin-provided HTML.
use axum::{
    extract::Query,
    http::header,
    response::{IntoResponse, Response},
};
use std::collections::HashMap;
const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'; object-src 'none'";
fn asset(body: &'static str, content_type: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "no-cache"),
            (header::CONTENT_SECURITY_POLICY, CSP),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::REFERRER_POLICY, "no-referrer"),
        ],
        body,
    )
        .into_response()
}
pub async fn index() -> Response {
    asset(
        include_str!("../web/index.html"),
        "text/html; charset=utf-8",
    )
}
pub async fn css() -> Response {
    asset(
        include_str!("../web/catalog.css"),
        "text/css; charset=utf-8",
    )
}
pub async fn js() -> Response {
    asset(
        include_str!("../web/catalog.js"),
        "text/javascript; charset=utf-8",
    )
}
pub async fn skill(Query(query): Query<HashMap<String, String>>) -> Response {
    let mut response = asset(
        include_str!("../skills/glint-plugin-dev/SKILL.md"),
        "text/plain; charset=utf-8",
    );
    if query.get("download").is_some_and(|v| v == "1") {
        response.headers_mut().insert(
            header::CONTENT_DISPOSITION,
            "attachment; filename=\"SKILL.md\"".parse().unwrap(),
        );
    }
    response
}

pub async fn icon() -> Response {
    asset(
        include_str!("../web/glint-api-icon.svg"),
        "image/svg+xml; charset=utf-8",
    )
}
