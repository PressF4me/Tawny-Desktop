> [!NOTE]
> **Built with AI.** Tawny is made by its maintainer working with Claude, an AI
> model made by Anthropic. Most of the code, documentation and artwork here was
> written with Claude, under the maintainer's direction. Some people avoid
> AI-built software for ethical, political, professional or personal reasons,
> so you should know that before you install, run or contribute.

# 🦉 Tawny for Linux

Tawny, the private two-way pet monitor, as a Linux desktop app. Point a webcam
at your pet (**Monitor**) or watch one from your computer (**Viewer**). It uses
the same web client as the Android app and pairs with it by QR code. No
account, no sign-up.

Video and audio go **peer to peer over WebRTC**, encrypted end to end
(DTLS-SRTP).

- **Android app:** [Tawny-Pet-Monitor-APK](https://github.com/PressF4me/Tawny-Pet-Monitor-APK)
- **Self-hosting:** [Tawny Docker](https://github.com/PressF4me/tawny)

## Install

| Format | Where |
|---|---|
| Flatpak | Flathub (`io.github.PressF4me.Tawny`, once published), or the `.flatpak` bundle from [Releases](https://github.com/PressF4me/Tawny-Linux/releases) |
| AppImage | [Releases](https://github.com/PressF4me/Tawny-Linux/releases): `chmod +x Tawny_*.AppImage` and run it |

x86_64 only for now.

## How it's built

The app is Rust: [Tauri 3](https://tauri.app) running on the **Chromium Embedded
Framework** (`tauri-runtime-cef`), not the usual WebKitGTK. Tawny is WebRTC
from end to end, and WebKitGTK's WebRTC is off or incomplete on most distros.
CEF is the same Chromium that Electron ships.

```
┌───────────── tawny (one process tree) ─────────────┐
│  CEF window ──http://127.0.0.1:47821──▶ loopback server (axum)
│   web/  (the Tawny web client)          ├─ web/, embedded at build time
│                                         ├─ /config.json  rendezvous, STUN
│                                         └─ /ws  signalling relay
└────────────────────────────────────────────────────┘
```

- **`src-tauri/src/server.rs`** serves the page over loopback, which is a
  secure context, so the camera and mic work. It answers only its own
  `Host`, and `/ws` answers only its own `Origin`, so other pages in your
  browser cannot use it.
- **`src-tauri/src/relay.rs`** is a Rust port of the relay in Tawny Docker's
  `server.js`: the same admission, tickets, close codes and caps. It is the
  page's same-origin relay of last resort.
- **`src-tauri/src/config.rs`** decides what the page is told about servers.
  The values are fixed at build time, like Android's gradle properties:

  | Variable | Default |
  |---|---|
  | `TAWNY_RENDEZVOUS_URL` | the same rendezvous as the Play Store app |
  | `TAWNY_STUN_URLS` | Cloudflare + Google STUN (`off` for none) |
  | `TAWNY_TURN_MODE` | `auto` (`always` forces TURN) |

- **`web/`** is vendored from the Android repo (`public/`), which is the
  source of truth. Refresh it with `tools/sync-web.sh`. `web/.source` records
  the commit.

The port stays at 47821 from run to run because the page keeps its saved
channels in `localStorage`, which is per origin, port included.

## Build

Requirements: Rust 1.95+, GTK 4.14+ dev headers, cmake, ninja, patchelf, and
the Tauri CLI that matches the pinned crates:

```sh
cargo install tauri-cli --version '=3.0.0-alpha.1' --locked
```

| | |
|---|---|
| Run from source | `cd src-tauri && cargo tauri dev` |
| AppImage | `tools/build-appimage.sh` → `dist/` |
| Flatpak | `tools/build-flatpak.sh` → installs for your user, bundle in `dist/` |

The first build downloads the CEF distribution (≈300 MB compressed) into
`~/.cache/tauri-cef`.

### Flatpak notes

- Flathub builds offline. Crates come from
  `packaging/flatpak/cargo-sources.json`; run `tools/update-cargo-sources.sh`
  and commit the result whenever `Cargo.lock` changes. CEF is the exact
  tarball `download-cef` would fetch, pinned by sha256 in the manifest. Bump
  it whenever the `cef` crate's version changes.
- Chromium's sandbox cannot create namespaces inside Flatpak, so the app runs
  under [zypak](https://github.com/refi64/zypak), like every Chromium-based
  app on Flathub.

## Releasing

Push a `v*` tag. `.github/workflows/release.yml` builds the AppImage and the
Flatpak bundle and attaches both to a GitHub release. Flathub builds from its
own `flathub/io.github.PressF4me.Tawny` repo, whose manifest is this one with
the `dir` source replaced by `type: git` and the tag.

## Status

Early. Tauri 3 and its CEF runtime are alpha, pinned exactly in `Cargo.toml`.

## License

MIT, see [`LICENSE`](LICENSE). The owlet mascot and the Tawny name are the
app's identity; please use your own if you publish a fork.
