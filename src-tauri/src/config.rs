//! What the page is told in `config.json`, and the CSP that follows from it.
//!
//! Fixed at build time, the way Android's `tawny.*` gradle properties are:
//!
//! ```text
//! TAWNY_RENDEZVOUS_URL=wss://…      the rendezvous both ends meet on off Wi-Fi
//! TAWNY_STUN_URLS=stun:…,stun:…     comma-separated; `off` for none
//! TAWNY_TURN_MODE=auto|always       auto = relay only when P2P fails
//! ```
//!
//! Unset, a build gets the same servers as the Play Store app. Set
//! `TAWNY_RENDEZVOUS_URL=` (empty) for a build that meets only on its own origin.

use serde_json::json;

const DEFAULT_RENDEZVOUS: &str = "wss://tawny-rendezvous.tawny1.workers.dev";
const DEFAULT_STUN: &str = "stun:stun.cloudflare.com:3478,stun:stun.l.google.com:19302";

pub struct Config {
  pub rendezvous: String,
  pub stun: Vec<String>,
  pub turn_mode: String,
}

impl Config {
  pub fn from_build() -> Self {
    let rendezvous = option_env!("TAWNY_RENDEZVOUS_URL")
      .unwrap_or(DEFAULT_RENDEZVOUS)
      .trim()
      .to_string();
    let stun = match option_env!("TAWNY_STUN_URLS").unwrap_or(DEFAULT_STUN).trim() {
      "off" => Vec::new(),
      s => list(s),
    };
    let turn_mode = match option_env!("TAWNY_TURN_MODE").unwrap_or("auto").trim() {
      "always" => "always",
      _ => "auto",
    }
    .to_string();
    Self { rendezvous, stun, turn_mode }
  }

  /// The body of `/config.json`, in the shape Android's build writes.
  pub fn json(&self) -> String {
    json!({
      "stun": self.stun,
      "rendezvous": self.rendezvous,
      "turnMode": self.turn_mode,
      "authRequired": false,
    })
    .to_string()
  }

  /// `connect-src`: this origin, plus the one rendezvous the page dials.
  ///
  /// Same reasoning as `CONNECT_SRC` in Tawny Docker's server.js: the directive
  /// exists so a script that got into the page cannot post the channel key to an
  /// arbitrary host, so it stays a list and never grows a scheme-wide source.
  pub fn connect_src(&self) -> String {
    let mut out = vec!["'self'".to_string()];
    let rv = bare_host(&self.rendezvous);
    if host_ok(rv) {
      // A plain ws:// rendezvous (a LAN or test server) is dialled as ws:// and
      // http://; a wss:// one would never match those, and the reverse.
      let (ws, http) = if self.rendezvous.starts_with("ws://") { ("ws", "http") } else { ("wss", "https") };
      out.push(format!("{ws}://{rv}"));
      out.push(format!("{http}://{rv}"));
    }
    // Belt and braces only — Chromium does not gate ICE servers on connect-src.
    for u in &self.stun {
      let h = bare_host(u);
      if host_ok(h) {
        out.push(format!("stun://{h}"));
        out.push(format!("turn://{h}"));
        out.push(format!("turns://{h}"));
      }
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|s| seen.insert(s.clone()));
    out.join(" ")
  }

  pub fn csp(&self) -> String {
    [
      "default-src 'none'",
      "script-src 'self'",
      "style-src 'self'",
      "img-src 'self' data: blob:",
      "media-src 'self' blob:",
      "font-src 'self'",
      &format!("connect-src {}", self.connect_src()),
      "manifest-src 'self'",
      "base-uri 'none'",
      "form-action 'none'",
      "frame-ancestors 'none'",
    ]
    .join("; ")
  }
}

fn list(v: &str) -> Vec<String> {
  v.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect()
}

/// `wss://host:port/path` -> `host:port`; `stun:host:3478` -> `host:3478`.
fn bare_host(u: &str) -> &str {
  let rest = match u.find(':') {
    Some(i) if u[..i].chars().all(|c| c.is_ascii_alphabetic()) => &u[i + 1..],
    _ => u,
  };
  let rest = rest.trim_start_matches("//");
  rest.split(['/', '?']).next().unwrap_or("")
}

/// A value that is not plainly `hostname[:port]` could truncate the policy.
fn host_ok(h: &str) -> bool {
  let (name, port) = match h.rsplit_once(':') {
    Some((n, p)) => (n, Some(p)),
    None => (h, None),
  };
  !name.is_empty()
    && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    && port.is_none_or(|p| (1..=5).contains(&p.len()) && p.chars().all(|c| c.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn hosts() {
    assert_eq!(bare_host("wss://a.example.dev/ws?x"), "a.example.dev");
    assert_eq!(bare_host("stun:stun.l.google.com:19302"), "stun.l.google.com:19302");
    assert!(host_ok("stun.l.google.com:19302"));
    assert!(!host_ok("evil.example; script-src *"));
    assert!(!host_ok(""));
  }

  #[test]
  fn connect_src_lists_rendezvous() {
    let c = Config {
      rendezvous: "wss://rv.example.dev".into(),
      stun: vec!["stun:s.example:3478".into()],
      turn_mode: "auto".into(),
    };
    let s = c.connect_src();
    assert!(s.starts_with("'self' wss://rv.example.dev https://rv.example.dev"));
    assert!(s.contains("stun://s.example:3478"));
  }

  #[test]
  fn connect_src_plain_ws_rendezvous() {
    let c = Config { rendezvous: "ws://192.168.1.5:8080".into(), stun: vec![], turn_mode: "auto".into() };
    assert_eq!(c.connect_src(), "'self' ws://192.168.1.5:8080 http://192.168.1.5:8080");
  }

  #[test]
  fn connect_src_no_repeats() {
    let c = Config {
      rendezvous: String::new(),
      stun: vec!["stun:a.example:3478".into(), "stun:b.example:3478".into(), "stun:a.example:3478".into()],
      turn_mode: "auto".into(),
    };
    assert_eq!(c.connect_src().matches("stun://a.example:3478").count(), 1);
  }
}
