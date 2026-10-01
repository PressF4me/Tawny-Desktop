//! Tawny for Linux and Windows: the Tawny web client in a Chromium (CEF) window, served by
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

  // Named after the app ID, not "tawny": on Windows %LOCALAPPDATA%\tawny is
  // the install folder itself (paths ignore case), and the NSIS uninstaller's
  // "delete app data" removes exactly %LOCALAPPDATA%\<identifier>. Local, not
  // roaming, data: a Chromium profile does not belong in %APPDATA%.
  // Under Flatpak these resolve inside ~/.var/app/io.github.PressF4me.Tawny.
  const ID: &str = "io.github.PressF4me.Tawny";
  let data = dirs::data_local_dir().unwrap_or_else(std::env::temp_dir).join(ID);
  let cache = dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join(ID);

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

      let (width, height) = window_size(app);
      WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
        .title("Tawny")
        .inner_size(width, height)
        .min_inner_size(360., 560.)
        .center()
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

/// 1024×760, or less where that would not fit: a 1366×768 laptop has about
/// 720 px of height left after the taskbar, and the window's own title bar
/// comes out of that too.
fn window_size(app: &tauri::App) -> (f64, f64) {
  const WANT: (f64, f64) = (1024., 760.);
  // Room for the title bar and window borders, which inner_size excludes.
  const FRAME: f64 = 48.;
  let Ok(Some(monitor)) = app.primary_monitor() else { return WANT };
  let area = monitor.work_area().size.to_logical::<f64>(monitor.scale_factor());
  (WANT.0.min(area.width - FRAME), WANT.1.min(area.height - FRAME))
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
