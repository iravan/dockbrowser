# DockBrowser — Universal App Runtime

Status: agreed direction, 2026-10-31. Extends `~/Downloads/dockbrowser_design_specification.md` (the original spec);
where the two disagree, this document wins. Evidence, prior art and open decisions: `RESEARCH.md`.

## 1. Goal

**Run any app on any device, as easily as opening a website.**

- **No platform limits.** Every app runs in the environment it was built for (Linux container, Windows VM,
  Android emulator, …), locally or on another machine. The device the user sits at is only a viewer.
- **Browser-easy.** Type an address and the app opens in a tab, with no install step. Close the tab and it's gone.
  Bookmarks, history and tabs work the way they do in a web browser.

The browser is redefined: DockBrowser is a **universal app viewer**. A website is one kind of app, and the web
is one environment among several.

Not goals: protecting users from phishing, or from malicious files they choose to open. Isolation is kept as
good hygiene (§9) but it isn't the product.

## 2. Browser concepts, mapped

| Web browser | DockBrowser |
|---|---|
| URL | **App address**: a web URL, `app:<name>`, an image ref, a manifest URL, or a local file (§4) |
| Website | **App**: a manifest that says which environment to start and what to run in it |
| Page load | **Environment start**: pull/build (first time), boot or restore a snapshot, connect the stream |
| Tab | **Running app session** (one app window or a whole desktop) |
| Close tab | Stop the environment and discard its state, unless the app has a saved profile |
| Bookmarks | **Pinned apps** |
| History | **Recent apps** |
| Cookies / profile | **Saved app data** (opt-in volume per app) |
| Downloads / uploads | **Files out / in** across the environment boundary |
| Web server | **Manifest host**: any https URL can serve an app manifest, with no store gatekeeping |
| Rendering engine | **Environment provider** (§5) |

## 3. Architecture

```
Viewer (any device)                     Orchestrator                     Environment providers
+---------------------------+          +----------------------+          +--------------------------------+
| Shell: address bar, tabs, |  IPC /   | resolve address      |  start/  | Linux container   (built)      |
|  pins, recents            |  HTTPS   |  -> manifest         |  stop    | Windows VM        (RDP)        |
| Tab = viewer canvas in a  | <------> | pick provider        | <------> | Android emulator               |
|  cross-origin iframe      |          | proxy + stream route |          | macOS VM   (Apple hosts only)  |
|  (stream client, host-    |  stream  | lifecycle, snapshots |  stream  | Remote host (any of the above  |
|   served)                 | <======> | warm pool            | <======> |  on another machine / cloud)   |
+---------------------------+          +----------------------+          +--------------------------------+
```

- **Shell = one web page.** The tab strip and address bar are ordinary UI, and each tab is a viewer canvas.
  Today's model of one OS window per tab goes away. Because the shell is a web page, the same UI can later be
  served to a phone or tablet for remote use.
- **Viewers run in cross-origin iframes** (`tab-N.dockbrowser.localhost`), never in the shell's origin, so a
  stream-decoder bug can't reach the shell's IPC.
- **The stream client (viewer JS) is served by the host**, never by the environment. Environments provide only
  the raw stream (VNC/RDP/WebRTC) over WebSocket through the proxy. *Today `vnc.html` is served from inside the
  container; fixing that is roadmap step 2.*

### Code mapping

- `src/main.rs`: orchestrator (tab ↔ environment lifecycle, reaper). Grows address resolution.
- `src/container.rs`: the Linux container provider. It stays the same; other providers sit beside it.
- `src/proxy.rs`: SNI-routed TLS proxy. It carries the stream WebSockets and the host-served viewer.
- `src/engine.rs`: host engines (WebKit/Blink/Gecko) become **direct mode**, a fast path for plain websites.
- `ui/index.html`: becomes the browser-like shell.

## 4. App addresses

The address bar decides what to do from what you type:

| Typed | Resolves to |
|---|---|
| `example.com`, `https://…` | Web app: direct mode (host engine) or a browser environment (container Firefox/Chromium) |
| `app:gimp`, `app:fedora` | A manifest from the built-in catalog |
| `app:https://host/x.json` | A manifest fetched over https |
| `ghcr.io/org/image`, `/path/with/Dockerfile` | Ad-hoc Linux app (today's custom-image path) |
| A local file (drag & drop / "Open with") | The app registered for that file type, with the file passed in |

### App manifest

A small JSON document, the app equivalent of a web page. Sketch:

```json
{
  "name": "GIMP",
  "platform": "linux",
  "source": "ghcr.io/example/gimp:3",
  "run": ["gimp"],
  "view": "window",
  "resources": { "cpus": "2", "memory": "2g" },
  "opens": ["image/png", "image/jpeg"],
  "persist": false
}
```

- `platform` picks the provider. `source` is an image, a Dockerfile, an installer URL or a VM template.
- `view` is `window` (a single app window, preferred) or `desktop`.
- The first open of a manifest from a new origin asks the user to confirm, as in the original spec's
  approval dialog. After that it opens like a website.

### Making it feel instant

- **Cache:** images and VM templates are pulled once and reused, like a browser cache.
- **Snapshots:** VMs restore from a booted snapshot instead of booting cold. A Windows cold boot takes minutes;
  restoring takes seconds.
- **Warm pool:** keep one pre-started environment per pinned app.
- **Progress in the tab:** pull, build and boot progress shows like a page-load bar, not a frozen button.

## 5. Environment providers

Common interface: `start(manifest) -> stream endpoint`, `stop()`, and optionally `snapshot()`.

| Provider | Runs | Stream | Feasibility |
|---|---|---|---|
| Linux container | Linux apps, browsers, desktops | VNC today → Xpra / WebRTC | ✅ Built |
| Windows VM | `.exe` apps | RDP **RemoteApp** (per-window, as WinApps/WinBoat do) | ✅ Needs a Windows license per VM. On this Mac it must run outside Docker (no `/dev/kvm` in the Rancher VM) |
| Wine in a container | Some Windows apps, light | Same as Linux | ⚠️ Only part of the catalogue works. x86 containers run emulated on this Mac, so this is the next spike |
| Android | APKs | scrcpy / VNC | ✅ Easy on Linux hosts, heavier on Mac |
| macOS VM | `.app` | VNC / Screen Sharing | ⛔ License-limited to Apple hardware, max 2 VMs per Mac |
| Remote host | Any of the above, elsewhere | Same protocols over the network | ✅ The real escape from local hardware, CPU type and OS limits |

**Hard limits (not solvable by engineering):** iOS apps outside Apple devices; macOS on non-Apple hardware;
games with anti-cheat and DRM video, which usually refuse to run in VMs.

## 6. Streaming

The central technical decision.

- **Per-window over full desktop:** an app should feel like a tab, not a desktop inside a frame.
  RDP RemoteApp (Windows) and Xpra (Linux) do this; VNC/noVNC can't.
- **One viewer, many protocols:** Apache Guacamole (`guacd`) translates VNC, RDP and SSH into a single web
  protocol and client. It's the strongest candidate to reuse instead of writing a viewer per protocol.
- **WebRTC** (Selkies, neko) for low latency, audio and video. It needs more machinery (UDP, more code);
  use it where VNC/RDP feel too slow.
- **Must-haves:** clipboard, audio, resize to the tab, keyboard layouts, HiDPI.

## 7. Integration (what makes it feel native)

- **Files:** a per-tab file tray for files in and out; "Open with" from Finder or Explorer resolves through
  manifest `opens`.
- **Clipboard:** sync on paste.
- **Saved app data:** opt-in named volume per app (logins and settings survive).
- **Notifications and window titles** surface in the tab strip.

## 8. Current state (already built)

- Tab open → container spawn, tab close → teardown. SNI-routed TLS proxy with a local CA. Host engines
  WebKit, Blink and Gecko.
- **Roadmap step 1 (first cut, 2026-10-31):**
  - One window: tab strip, address bar (back/forward/reload, pin, engine picker), start page (apps, pins,
    recents, custom container).
  - WebKit tabs and app viewers are child webviews below the toolbar. Blink/Gecko remain external windows.
  - `app:firefox`, `app:fedora` from the compiled-in `APPS` catalog (`apps/<name>/Dockerfile`). The engine
    choice "container" opens a web URL in `app:firefox`.
  - Load progress is a "Starting…" tab only (no build log yet).
  - Orphaned containers (dead owner pid) are removed at startup.
- `apps/firefox`: Debian Firefox ESR + Xvfb + x11vnc + noVNC, URL via `DOCKBROWSER_URL`.
  Live test: `container::tests::browser_container`.
- `apps/fedora`: Fedora 44 XFCE desktop. Verified to boot under the container restrictions.
- Every container gets `--shm-size=256m`, because the 64m default crashes browser tabs.

## 9. Isolation (hygiene, not the product)

Kept because it's cheap and prevents environments from breaking each other or the host:

- The existing invariants (§11) stay.
- Host-served viewer plus a per-session stream token (today noVNC has no auth on the loopback port).
- Default network policy: environments get internet access but not host or LAN access, unless a manifest asks
  for it.
- One VM per environment where the platform offers it cheaply (Apple Containerization on macOS 26, Kata/Firecracker on Linux).

## 10. Roadmap

1. **Browser-like shell**
   - One window with an address bar, tab strip and viewer iframes, replacing the launcher and per-tab windows.
   - `app:` addresses backed by built-in manifests (`app:firefox`, `app:fedora`).
   - Recents and pins.
   - Load-progress UI.
   - Leftover containers removed on startup (`--label dockbrowser`).
2. **Viewer**
   - Serve the stream client from the host.
   - Spike Guacamole vs Xpra for per-window Linux apps.
   - Clipboard and resize.
3. **Windows provider spike**
   - One Windows VM, one app shown per-window through RDP RemoteApp in a tab.
   - Snapshot restore for fast start.
4. **Remote provider**
   - Run environments on another machine.
   - Serve the shell as a plain web page so phones and tablets work.
5. **Integration**
   - File tray, "Open with", saved app data.
6. **Android provider.**
7. **Manifest hosting**
   - `app:https://…` manifests, a first-open confirmation, and a starter catalog.

## 11. Invariants (unchanged from AGENTS.md)

- Every Tauri command calls `require_shell`; IPC is never widened to tab or viewer frames.
- URLs go through `parse_url` (http/https only); nothing handed to child-process argv starts with `-`.
- Containers: loopback-only ephemeral ports, `--cap-drop=ALL`, `no-new-privileges`, CPU/memory limits always set.
- Tabs reach environments only through the proxy.
- The CA stays name-constrained to `dockbrowser.localhost`; the proxy stays HTTP/1.1.

## 12. Accepted trade-offs

- **Resources:** a Windows VM needs 4–8 GB of RAM, a Linux browser environment 0.5–1 GB.
  Remote providers are the answer for small devices.
- **No GPU** in VMs on macOS, so graphics-heavy apps should use a remote provider with a GPU.
- **Latency:** every interaction goes through a stream. It's fine for productivity apps but not for fast games.
- **Licensing:** Windows needs a license per VM, and macOS environments are only allowed on Apple hardware.

## 13. Open questions

- Shell in Tauri only, or also served as a web page from day one?
- Guacamole (one gateway, many protocols) vs a native client per protocol?
- Windows VM tooling on macOS hosts: UTM/QEMU, or Apple Virtualization.framework (ARM Windows only)?
- Manifest trust: is first-open confirmation enough, or do manifests need signing?
- Runtime: Podman rootless (original spec) vs Docker via Rancher Desktop (used today)?
