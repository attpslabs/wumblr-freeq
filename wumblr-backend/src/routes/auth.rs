//! Dev-only OAuth callback bridge.
//!
//! ATProto OAuth requires every `redirect_uri` in a discoverable client's
//! metadata to be either HTTPS-on-the-client_id-origin or a reverse-FQDN
//! custom scheme. Loopback addresses (`http://127.0.0.1:*`) are NOT
//! permitted for hosted-metadata clients — only the special
//! `client_id=http://localhost` development workflow allows them.
//!
//! So we run the OAuth dance against the production redirect
//! (`https://api.wumblr.com/auth/callback`) even from dev, and bounce the
//! browser back to the dev origin from this handler.
//!
//! Behavior:
//!   GET /auth/callback?<all query params>
//!       (+ optional `#state=...&code=...` fragment from PDS)
//!   → 302 to `WUMBLR_DEV_CALLBACK_TARGET` + same query string + fragment
//!     when WUMBLR_DEV_CALLBACK_TARGET is set
//!   → otherwise, simple page that JS-redirects to the same URL
//!     (fragment-preservation requires client-side JS because servers
//!     don't see fragments)
//!
//! Production deployment: leave WUMBLR_DEV_CALLBACK_TARGET unset and
//! deploy a real Cloudflare Pages frontend that consumes
//! https://api.wumblr.com/auth/callback directly via this handler.

use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use std::collections::BTreeMap;

use crate::app::AppState;

#[derive(Debug, Deserialize)]
pub struct CallbackParams {
    #[serde(flatten)]
    pub all: BTreeMap<String, String>,
}

pub async fn callback(
    State(state): State<AppState>,
    Query(params): Query<CallbackParams>,
    _headers: HeaderMap,
) -> Response {
    // The PDS often sends OAuth params as a URL *fragment* (`#code=...`)
    // rather than a query string. Servers don't see fragments — only the
    // browser does. So our HTML response runs a tiny JS shim that:
    //   1. Reads window.location.{search,hash}
    //   2. Constructs the dev-target URL with both preserved
    //   3. Redirects there
    //
    // If params arrived as a query string AND we have a dev target, we
    // can also do a server-side 302 as a fast path.

    let dev_target = state.config.dev_callback_target.as_deref();

    let qs = encode_query(&params.all);

    match dev_target {
        Some(target) if !params.all.is_empty() => {
            let dest = if qs.is_empty() {
                target.to_string()
            } else {
                format!("{target}?{qs}")
            };
            Redirect::to(&dest).into_response()
        }
        Some(target) => {
            // No query params on the server side. Browser may have a fragment
            // we need to preserve via JS.
            Html(bridge_html(target)).into_response()
        }
        None => {
            // Production-style request — no dev bridge configured. Return a
            // friendly note explaining the user is at the wrong URL. The
            // production frontend at wumblr.com will replace this handler
            // entirely once Cloudflare Pages ships.
            (
                StatusCode::NOT_FOUND,
                [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                "wumblr-backend: /auth/callback is a dev bridge. Set \
                 WUMBLR_DEV_CALLBACK_TARGET on the backend to enable, or \
                 deploy the frontend so it can serve this URL directly.",
            )
                .into_response()
        }
    }
}

fn encode_query(params: &BTreeMap<String, String>) -> String {
    let mut out = String::new();
    for (i, (k, v)) in params.iter().enumerate() {
        if i > 0 {
            out.push('&');
        }
        out.push_str(&urlencode(k));
        out.push('=');
        out.push_str(&urlencode(v));
    }
    out
}

fn urlencode(s: &str) -> String {
    // Minimal application/x-www-form-urlencoded subset — enough for OAuth
    // state/code/iss values which are URL-safe to start with.
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn bridge_html(target: &str) -> String {
    // Inline JS reads search + hash from the current URL and forwards to
    // the dev target with both preserved. Wrapped in an HTML doc so the
    // browser parses correctly (no plain JS response).
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>wumblr auth bridge</title>
</head>
<body>
  <p>Returning you to the local app…</p>
  <script>
    (function() {{
      var target = {target_js};
      var search = window.location.search || "";
      var hash = window.location.hash || "";
      window.location.replace(target + search + hash);
    }})();
  </script>
</body>
</html>"#,
        target_js = escape_js_string(target),
    )
}

fn escape_js_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '<' => out.push_str("\\u003c"), // defensive against </script>
            '>' => out.push_str("\\u003e"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urlencode_passes_unreserved() {
        assert_eq!(urlencode("AZaz09-_.~"), "AZaz09-_.~");
    }

    #[test]
    fn urlencode_escapes_others() {
        assert_eq!(urlencode("a=b&c d"), "a%3Db%26c%20d");
    }

    #[test]
    fn encode_query_sorted_pairs() {
        let mut p = BTreeMap::new();
        p.insert("state".to_string(), "abc".to_string());
        p.insert("code".to_string(), "xyz".to_string());
        // BTreeMap iterates in sorted order: code, state
        assert_eq!(encode_query(&p), "code=xyz&state=abc");
    }

    #[test]
    fn js_string_escapes_quotes_and_html_brackets() {
        // </script> defenses: < and > become unicode-escaped so they can't
        // close the surrounding <script> block when embedded in HTML.
        assert_eq!(
            escape_js_string("</script>foo\"bar"),
            "\"\\u003c/script\\u003efoo\\\"bar\""
        );
    }
}
