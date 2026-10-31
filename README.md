# DockBrowser

**Run any app as easily as opening a website.**

DockBrowser is a desktop app (Rust + Tauri v2) that looks and works like a browser, but a tab can hold more than
a web page. Type an address and you get:

- **A website**, rendered by WebKit inside the window, or by Chrome (Blink) or Firefox (Gecko) in their own window.
- **An app running in a container**, such as a Firefox or a whole Fedora desktop, streamed into the tab.
- **Your own container image**, served to the tab through a local HTTPS proxy.

Close the tab and its container is stopped and deleted.

> **Status: early prototype.** The core works (tabs ↔ containers, TLS proxy, Firefox and Fedora apps). The
> browser-style one-window UI is new and hasn't been checked visually yet. macOS is the primary platform.
> The direction is in [`DESIGN.md`](DESIGN.md), and the research behind it is in [`RESEARCH.md`](RESEARCH.md).

## Quick start

**Requirements**

- macOS (Apple Silicon or Intel)
- [Rust](https://rustup.rs) (stable)
- A container runtime: Docker (for example Rancher Desktop or Docker Desktop) or Podman, **running**
- Optional, for the `blink` / `gecko` engines: Google Chrome or Chromium, and Firefox

```sh
cargo run
```

On first launch, macOS asks for your password once. That adds DockBrowser's local certificate authority to
your login keychain, so container tabs open over HTTPS without warnings.

## Using it

### The address bar

| Type | You get |
|---|---|
| `example.com` or `https://…` | The site, in the engine chosen next to the address bar |
| `app:firefox` | Firefox running in a container |
| `app:fedora` | A Fedora XFCE desktop running in a container |

The **first open of an app builds its image**, which takes a few minutes. Later opens take seconds.

### Engines

| Engine | Where it renders |
|---|---|
| `auto` | The engine you chose for that site before, otherwise WebKit |
| `webkit` | Inside the DockBrowser window |
| `blink` | A separate Chrome/Chromium window with a throwaway profile |
| `gecko` | A separate Firefox window with a throwaway profile |
| `container (Firefox)` | The site opens in `app:firefox`, so the page never runs on your Mac |

On an open web tab, changing the engine moves that tab to the new engine. Tick **Always** to remember the
engine for that site; preferences are saved to `prefs.json`.

### Toolbar and start page

- **◀ ▶ ⟳** go back, forward and reload (WebKit tabs).
- **☆** pins the current tab to the start page.
- **+** opens the start page: built-in apps, pinned tabs, recent tabs, and **Custom container**.

### Custom container

On the start page, open **Custom container** and fill in:

- **Image or Dockerfile directory:** an image reference (`nginxinc/nginx-unprivileged`), or a local folder that
  contains a `Dockerfile`, which gets built.
- **Port:** the port the container serves HTTP on (`8080` for the example above).
- **CPUs / Memory:** the resource limits.

The tab opens `https://tab-<n>.dockbrowser.localhost:<port>/`, which proxies to the container.

Images must be able to run as a **non-root user with no Linux capabilities** (see [Security](#security)). For
example, `nginx` fails but `nginxinc/nginx-unprivileged` works.

## Adding a built-in app

1. Create `apps/<name>/Dockerfile`. It must serve something over HTTP on one port. The existing apps run Xvfb, x11vnc and noVNC
   on port `6080`; use [`apps/fedora/Dockerfile`](apps/fedora/Dockerfile) as a template.
2. Add one entry to `APPS` in [`src/main.rs`](src/main.rs):
   ```rust
   App { name: "myapp", dockerfile: include_str!("../apps/myapp/Dockerfile"), port: 6080, viewer: NOVNC, memory: "2g" },
   ```
3. Rebuild. The app is now available as `app:myapp` and appears on the start page.

## How it works

```
DockBrowser window
  toolbar (tabs, address bar)        ← shell webview: the only one allowed to call the app's commands
  tab area: child webview per tab    ← untrusted content, no IPC
        │ https://tab-N.dockbrowser.localhost
        ▼
  local TLS proxy (routes by SNI) ──► 127.0.0.1:<random port> ──► container (one per tab)
```

| File | Role |
|---|---|
| `src/main.rs` | Orchestrator: address resolution, app catalog, tab lifecycle, Tauri commands, cleanup |
| `src/engine.rs` | Render engines: WebKit (in-window), Blink/Gecko (external processes) |
| `src/container.rs` | Docker/Podman CLI adapter: build, run with limits, readiness check, stop, orphan cleanup |
| `src/proxy.rs` | TLS reverse proxy plus the local certificate authority |
| `ui/index.html` | The shell UI (plain HTML/JS, no build step) |
| `apps/*/Dockerfile` | Built-in apps |

## Security

- **IPC is locked to the shell.** Every command checks that the caller is the shell webview, and the
  capability is scoped to that webview only. Tabs (untrusted content) get no IPC.
- **Containers are locked down:**
  - no Linux capabilities (`--cap-drop=ALL`) and `no-new-privileges`
  - CPU, memory and `/dev/shm` limits always set
  - ports bound to loopback only, and tabs reach a container only through the proxy
- **The local certificate authority can only sign names under `dockbrowser.localhost`.** If its key leaked, it
  still couldn't be used to impersonate other sites.
- **Leftover containers are removed:** each container is labelled with the PID of the app instance that
  started it. Containers whose owning instance has died are removed at the next startup, while other running
  instances keep theirs.

Known gaps:
- noVNC has no access token yet, so any local process can reach a container's loopback port.
- Containers can reach your LAN and host services through Docker's default network.
- The noVNC viewer page is served from inside the container.

All three are on the roadmap in [`DESIGN.md`](DESIGN.md).

## Limitations

- **Platform:** certificate trust is macOS-only (keychain). Windows isn't supported, and on Linux container tabs
  show certificate errors.
- **Streamed apps:** no clipboard sync, no audio, no GPU, and the screen is a fixed 1280×800, scaled to fit.
- **Saved data:** none yet. Everything in a tab is gone when it closes.
- **Windows apps:** not yet. The plan and the options are in [`RESEARCH.md`](RESEARCH.md) §4.
- **App build progress:** a first open shows "Starting…" with no build log.

## Development

```sh
cargo test                           # unit tests
cargo test -- --include-ignored      # also live container tests (needs Docker/Podman running)
```

Conventions for contributors and AI agents are in [`AGENTS.md`](AGENTS.md).

**Housekeeping**

- **App data** (CA, `prefs.json`, app build contexts): `~/Library/Application Support/dev.dockbrowser.app/`
- **Remove CA trust:** `security delete-certificate -c "DockBrowser Local CA"`
- **List DockBrowser containers:** `docker ps --filter label=dockbrowser`

## Docs

- [`DESIGN.md`](DESIGN.md): the direction (universal app runtime used like a browser) and the roadmap
- [`RESEARCH.md`](RESEARCH.md): prior art, whether it's worth it, Windows options, security findings
- [`AGENTS.md`](AGENTS.md): codebase map and conventions
