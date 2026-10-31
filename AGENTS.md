# dockbrowser

Desktop browser (Rust, Tauri v2) that opens each tab in a chosen render engine (WebKit / Blink / Gecko)
and can attach a container backend per tab: tab open → container spawn, tab close → container teardown.
Design spec: `~/Downloads/dockbrowser_design_specification.md`. Phases 1–2 done.
Direction is now a **universal app runtime used like a browser** (any app runs in its native environment, the
device only views; app addresses, tabs, pins): see `DESIGN.md`, which overrides the spec.
Research, prior art and open decisions (incl. a checklist for the user): `RESEARCH.md`. Roadmap step 1 is coded but not yet
checked visually; next candidate is a Wine spike (`app:notepadpp`).

## Commands

- `cargo run` — launch the app
- `cargo test` — unit tests
- `cargo test -- --include-ignored` — also runs the live container test (needs podman or docker running)

## Layout

- `src/main.rs` — shell + orchestrator: Tauri commands, address resolution (`resolve`), `APPS` catalog, tab table, domain engine prefs, teardown reaper
- `src/engine.rs` — Render Engine Abstraction Layer: `RenderEngine` / `EngineTab` traits, `ENGINES` registry.
  WebKit tabs are child webviews of the `main` window (tauri `unstable` feature); Blink/Gecko are external windows
- `src/container.rs` — Container Runtime Adapter: podman/docker CLI, build/run/stop, readiness probe
- `src/proxy.rs` — TLS reverse proxy: `https://tab-<id>.dockbrowser.localhost:<port>` → container, routed by SNI; local CA + keychain trust
- `apps/<name>/Dockerfile` — built-in apps (`app:firefox`, `app:fedora`): Xvfb + noVNC on :6080, embedded with `include_str!`.
  Firefox takes its URL from `DOCKBROWSER_URL`. New app = one Dockerfile + one `APPS` entry
- `ui/index.html` — shell UI (plain HTML, no build step): tab strip, address bar, start page; pins/recents in localStorage.
  `#toolbar` height must equal `engine::TOOLBAR_HEIGHT`
- `capabilities/default.json` — IPC permissions, scoped to the `main` **webview** (tab webviews share its window,
  so never scope by window)

## Conventions

- Engine tabs load untrusted content. Every Tauri command calls `require_shell` first; never widen IPC to tab webviews.
  The shell renders tab titles/URLs with `textContent` only, never `innerHTML`.
- Navigation URLs go through `parse_url` (http/https only). Values handed to child-process argv must never start with `-`.
- Containers: loopback-only ephemeral ports, `--cap-drop=ALL`, `no-new-privileges`, CPU/memory limits are always set.
  Each is labelled `dockbrowser=<app pid>`; startup removes those whose owner process is dead.
  Tabs reach containers only through the proxy URL, never the raw container port.
- The local CA is name-constrained to `dockbrowser.localhost`; keep it that way. Proxy stays HTTP/1.1 (no ALPN h2)
  so browsers can't coalesce different tab hosts onto one connection.
- App data (CA, `prefs.json`): `~/Library/Application Support/dev.dockbrowser.app/`.
  Remove CA trust: `security delete-certificate -c "DockBrowser Local CA"`.
- `ponytail:` comments mark deliberate shortcuts and how to upgrade them.
- New engine = one more entry in `ENGINES`.
