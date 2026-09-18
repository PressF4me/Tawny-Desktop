//! The loopback server the window loads the web client from.
//!
//! Tawny's page is served over `http://127.0.0.1:<port>` rather than Tauri's own
//! protocol, for the same reason Android's LocalWeb.kt does it: loopback is a
//! secure context, so getUserMedia works, and the page's relay of last resort
//! is "the origin I came from" — which only works if that origin speaks `/ws`.
//!
//! The port is kept the same from run to run where it can be, because the
//! page's saved channels live in localStorage, which is per origin, port
//! included. A different port is a different, empty, localStorage.

use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::sync::Arc;

use axum::Router;
use axum::extract::{Query, State, WebSocketUpgrade};
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::Deserialize;
use tauri::{AppHandle, Runtime};

use crate::config::Config;
use crate::relay::{self, Relay, Role};

/// First choice of port, and how far to look past it.
const PREFERRED_PORT: u16 = 47821;
const PORT_TRIES: u16 = 20;

struct Ctx<R: Runtime> {
  app: AppHandle<R>,
  origin: String,
  host: String,
  csp: String,
  connect_src: String,
  config_json: String,
  relay: Relay,
}

/// Bind now, so the caller knows the address before the window is built, and
/// serve on Tauri's async runtime.
pub fn start<R: Runtime>(app: AppHandle<R>) -> std::io::Result<u16> {
  let listener = bind()?;
  let port = listener.local_addr()?.port();
  if port != PREFERRED_PORT {
    log::warn!("port {PREFERRED_PORT} is taken; serving on {port}, so saved channels will not show this run");
  }
  listener.set_nonblocking(true)?;

  let config = Config::from_build();
  let ctx = Arc::new(Ctx {
    app,
    origin: format!("http://127.0.0.1:{port}"),
    host: format!("127.0.0.1:{port}"),
    csp: config.csp(),
    connect_src: config.connect_src(),
    config_json: config.json(),
    relay: Relay::default(),
  });

  tauri::async_runtime::spawn(ctx.relay.clone().upkeep());
  tauri::async_runtime::spawn(async move {
    let router = Router::new()
      .route("/config.json", get(config_json::<R>))
      .route("/ws", get(ws::<R>))
      .fallback(get(asset::<R>))
      .with_state(ctx);
    let listener = match tokio::net::TcpListener::from_std(listener) {
      Ok(l) => l,
      Err(e) => return log::error!("loopback server: {e}"),
    };
    if let Err(e) = axum::serve(listener, router).await {
      log::error!("loopback server stopped: {e}");
    }
  });
  Ok(port)
}

fn bind() -> std::io::Result<TcpListener> {
  let mut last = None;
  for port in PREFERRED_PORT..PREFERRED_PORT + PORT_TRIES {
    match TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port))) {
      Ok(l) => return Ok(l),
      Err(e) => last = Some(e),
    }
  }
  Err(last.unwrap())
}

/// Only this app's own page may use this server. The Host check keeps a DNS
/// rebinding page out; the Origin check (on /ws) keeps out every other page
/// the user has open in a browser, which can all reach 127.0.0.1 too.
fn host_ok<R: Runtime>(ctx: &Ctx<R>, h: &HeaderMap) -> bool {
  h.get(header::HOST).is_some_and(|v| v.as_bytes() == ctx.host.as_bytes())
}

fn origin_ok<R: Runtime>(ctx: &Ctx<R>, h: &HeaderMap) -> bool {
  h.get(header::ORIGIN).is_some_and(|v| v.as_bytes() == ctx.origin.as_bytes())
}

fn secure(mut res: Response, csp: &str) -> Response {
  let h = res.headers_mut();
  if let Ok(v) = HeaderValue::from_str(csp) {
    h.insert(header::CONTENT_SECURITY_POLICY, v);
  }
  h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
  h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
  h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
  res
}

async fn config_json<R: Runtime>(State(ctx): State<Arc<Ctx<R>>>, h: HeaderMap) -> Response {
  if !host_ok(&ctx, &h) {
    return StatusCode::MISDIRECTED_REQUEST.into_response();
  }
  let res = ([(header::CONTENT_TYPE, "application/json; charset=utf-8")], ctx.config_json.clone());
  secure(res.into_response(), &ctx.csp)
}

async fn asset<R: Runtime>(State(ctx): State<Arc<Ctx<R>>>, h: HeaderMap, uri: Uri) -> Response {
  if !host_ok(&ctx, &h) {
    return StatusCode::MISDIRECTED_REQUEST.into_response();
  }
  let path = match uri.path() {
    "/" => "/index.html",
    p => p,
  };
  // Tauri embeds web/ at build time (frontendDist); nothing is read from disk.
  let resolver = ctx.app.asset_resolver();
  let Some(asset) = resolver.get(path.to_string()) else {
    return StatusCode::NOT_FOUND.into_response();
  };
  // The resolver answers an unknown path with index.html, like an SPA host.
  // This page is not one: a missing file is a 404, not the app again.
  if path != "/index.html" && resolver.get("/index.html".into()).is_some_and(|i| i.bytes == asset.bytes) {
    return StatusCode::NOT_FOUND.into_response();
  }
  let mut body = asset.bytes;
  if path == "/index.html" {
    let html = String::from_utf8_lossy(&body).replace("__TAWNY_CONNECT_SRC__", &ctx.connect_src);
    body = html.into_bytes();
  }
  let mime = mime(path).map_or(asset.mime_type, String::from);
  secure(([(header::CONTENT_TYPE, mime)], body).into_response(), &ctx.csp)
}

#[derive(Deserialize)]
struct WsQuery {
  room: Option<String>,
  role: Option<String>,
}

async fn ws<R: Runtime>(
  State(ctx): State<Arc<Ctx<R>>>,
  h: HeaderMap,
  Query(q): Query<WsQuery>,
  up: WebSocketUpgrade,
) -> Response {
  if !host_ok(&ctx, &h) {
    return StatusCode::MISDIRECTED_REQUEST.into_response();
  }
  if !origin_ok(&ctx, &h) {
    return StatusCode::FORBIDDEN.into_response();
  }
  if ctx.relay.full() {
    return StatusCode::SERVICE_UNAVAILABLE.into_response();
  }
  let room = q.room.unwrap_or_default();
  if !relay::room_ok(&room) {
    return StatusCode::BAD_REQUEST.into_response();
  }
  let role = Role::parse(q.role.as_deref().unwrap_or(""));
  let relay = ctx.relay.clone();
  up.max_message_size(relay::MAX_MSG)
    .on_upgrade(move |socket| relay.serve(socket, room, role))
}

/// server.js's MIME table. Tauri's guess has no charset and calls a
/// .webmanifest HTML.
fn mime(path: &str) -> Option<&'static str> {
  Some(match path.rsplit_once('.')?.1 {
    "html" => "text/html; charset=utf-8",
    "css" => "text/css; charset=utf-8",
    "js" | "mjs" => "text/javascript; charset=utf-8",
    "json" => "application/json; charset=utf-8",
    "webmanifest" => "application/manifest+json; charset=utf-8",
    "svg" => "image/svg+xml",
    "png" => "image/png",
    "ogg" | "oga" => "audio/ogg",
    "mp3" => "audio/mpeg",
    "woff2" => "font/woff2",
    "ico" => "image/x-icon",
    _ => return None,
  })
}
