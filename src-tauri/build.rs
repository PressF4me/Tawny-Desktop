fn main() {
  // The servers the page is told about in config.json, fixed at build time the
  // way Android's gradle properties are. See src/config.rs.
  for var in ["TAWNY_RENDEZVOUS_URL", "TAWNY_STUN_URLS", "TAWNY_TURN_MODE"] {
    println!("cargo:rerun-if-env-changed={var}");
  }
  tauri_build::build()
}
