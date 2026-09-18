//! Tawny for Linux: the Tawny web client in a Chromium (CEF) window, served by
//! a small Rust server on loopback. See README.md for the shape of it.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod relay;
mod server;

use tauri::webview::{NewWindowResponse, PermissionKind, PermissionResponse};
use tauri::{Url, WebviewUrl, WebviewWindowBuilder};
use tauri_runtime_cef::{AutoplayPolicy, Cef};

/// The same executable is also Chromium's renderer, GPU and utility process.
/// The attribute runs that side and returns before the app is ever built.
#[tauri_runtime_cef::cef_entry_point]
fn main() {
  env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

  // Under Flatpak these resolve inside ~/.var/app/io.github.PressF4me.Tawny.
  let data = dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("tawny");
  let cache = dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("tawny");

  let cef = Cef::default()
    // The Chromium profile: localStorage (saved channels, theme, language) and
    // the permission decisions below live here, so it has to outlast the run.
    .root_cache_path(data.join("profile"))
    .log_file(cache.join("cef.log"))
    // The Monitor rings a chime the Viewer asked for, with nobody at the
    // keyboard to click first. The window only ever shows Tawny's own page.
    .autoplay(AutoplayPolicy::NoUserGestureRequired);

  tauri::Builder::default()
    .runtime(cef)
    .setup(|app| {
      let port = server::start(app.handle().clone())?;
      let origin = format!("http://127.0.0.1:{port}");
      let url: Url = format!("{origin}/").parse()?;

      WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
        .title("Tawny")
        .inner_size(1024., 760.)
        .min_inner_size(360., 560.)
        .on_permission_request(|_webview, kind| match kind {
          // Nothing but Tawny's page ever loads here (see on_navigation), and
          // being a camera is the whole job.
          PermissionKind::Camera | PermissionKind::Microphone => PermissionResponse::Allow,
          _ => PermissionResponse::Default,
        })
        // Links out of the app — Ko-fi, the privacy policy, GitHub — go to the
        // user's browser. The window never leaves this origin.
        .on_navigation({
          let origin = origin.clone();
          move |url| {
            if same_origin(url, &origin) {
              return true;
            }
            open_external(url);
            false
          }
        })
        .on_new_window(|url, _features| {
          open_external(&url);
          NewWindowResponse::Deny
        })
        .build()?;
      Ok(())
    })
    .run(tauri::generate_context!())
    .expect("error while running Tawny");
}

fn same_origin(url: &Url, origin: &str) -> bool {
  url.origin().ascii_serialization() == origin
}

fn open_external(url: &Url) {
  if matches!(url.scheme(), "https" | "http" | "mailto" | "lightning") {
    // Under Flatpak this goes through the OpenURI portal.
    if let Err(e) = open::that_detached(url.as_str()) {
      log::warn!("could not open {url}: {e}");
    }
  }
}
