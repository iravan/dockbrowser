# DockBrowser — Research & Findings

Collected 2026-10-31. Companion to `DESIGN.md` (the agreed direction). This file is the evidence and the open
decisions behind it.

> Prior-art lists are from the assistant's knowledge, **not a fresh web search**. Re-check for recent projects
> before publishing or pitching.

---

## 1. To decide when you're back

- [ ] **Try roadmap step 1 in the app.** Quit any running old instance, then `cargo run`:
  1. Type `example.com`. It should load **inside the window** below the toolbar, and back/forward/reload should work.
  2. Open `app:fedora`. You should see a "⏳ Starting…" tab, then the desktop.
  3. Switch tabs and resize the window. The page area should track the window.
  4. Pin a tab, then click **+**. The pin and the recent item should show on the start page.

  This has never been checked by eye; the screenshot was blocked by macOS permissions. The riskiest part is
  attaching child webviews to the config-created `main` window (tauri `unstable` feature).
- [ ] **Pick the goal:** learning / a tool for yourself / open source / product (see §3).
- [ ] **Pick one narrow use case** to make excellent (see §3).
- [ ] **Pick a Windows app** for the Wine spike, or default to Notepad++ (see §4).

---

## 2. Is it a unique project?

**No, not as a whole idea; each piece exists. The combination is uncommon, especially as a local desktop app.**

### Closest overall

| Project | Overlap | Difference |
|---|---|---|
| **ChromeOS** | Closest *concept*: browser as the OS, Linux apps in containers (Crostini), Android in a VM, Windows via Parallels | A whole OS tied to Chromebook hardware, not an app on Mac/Windows |
| **Kasm Workspaces** | Closest *product*: apps and browsers in Docker, streamed to a tab, with an app catalog ("Workspaces Registry") | Server/enterprise focused; a web portal, not a browser-like desktop app |
| **Lunascape** (2000s) | Per-tab engine switching (Trident / Gecko / WebKit) | The original spec's "engine per tab" idea, done about 15 years ago |

### Individual parts that already exist

- **Browser in a container:** neko, linuxserver.io Firefox, Cloudflare / Menlo remote browser isolation.
- **Desktop in a browser tab:** linuxserver.io Webtop, Apache Guacamole, Selkies.
- **Windows apps on another OS:** WinApps, WinBoat (RDP RemoteApp), dockur/windows, CrossOver/Wine.
- **Disposable sandboxes:** Qubes OS, Windows Sandbox.
- **Cloud app streaming:** Amazon AppStream, Citrix, Azure Virtual Desktop, Shadow, Parsec.
- **Everything in the browser via WebAssembly (a different route):** WebVM/CheerpX, v86, container2wasm.

### What could set it apart

No single product has all of:
1. **Local-first desktop app:** one download, no server, works on a Mac.
2. **Browser-easy:** type an address, an app opens in a tab, pin it, close it and it's gone.
3. **Mixed tabs in one strip:** websites (fast, on the host), Linux containers, Windows VMs, remote machines.
4. **Open manifests over https:** anyone can publish an app like a website, with no store gatekeeping.

Short version: **"the ChromeOS/Kasm experience as a local app on any desktop."**

---

## 3. Does the project have a point?

**Yes. Its value depends on the goal.** Others existing doesn't make it pointless (Chrome came after Firefox,
Slack after IRC).

### The gap it fills

Every alternative asks for something heavy: Kasm needs a server, ChromeOS needs a Chromebook, Qubes needs a
switch to Linux, and Parallels/UTM mean managing whole VMs. Nothing does "type `app:gimp` on your Mac and it
opens in a tab, with nothing to install". The prototype already does a rough version.

| Goal | Verdict |
|---|---|
| Learning | Already high value: Rust, Tauri, TLS/CA, container security, streaming |
| Tool for yourself | Yes, if you'd actually use it. Firefox and Fedora already run in tabs |
| Open source | Probably. Real niche: developers and tinkerers wanting throwaway Linux/Windows apps on a Mac |
| Startup / product | Unproven. Needs a sharper reason to exist than "runs everything" against Kasm, Parallels and ChromeOS |

### Risk

The risk is breadth, not pointlessness. "Run every app on every platform" is a multi-year job for a big team,
and side projects stall when they try to do everything at once. The hard parts (smooth streaming, native-feeling
Windows windows, fast start) are where Kasm, Parsec and WinApps have invested years.

### Recommendation

1. Pick **one narrow use case** and make it excellent. Candidates:
   - **Linux apps on a Mac, as tabs:** for developers who want GIMP, Linux CLIs or a Fedora/Ubuntu desktop without setting up a VM.
   - **Windows apps on a Mac as single windows:** Parallels charges for this, so a free, light version would get attention.
   - **Throwaway dev environments as tabs:** open a repo address, get a ready environment.
2. **Reuse, don't rebuild:** KasmVNC, Selkies, Guacamole and dockur/windows can become providers.
3. **Validate:** show it to five people who have the problem. Ask what they use today and whether they'd switch.

---

## 4. Running Windows apps

### Measured on this Mac (2026-10-31)

| Check | Result | Meaning |
|---|---|---|
| Host CPU | Apple Silicon (images build as `aarch64`) | Native containers are ARM |
| Container runtime | Docker via Rancher Desktop; VM is Ubuntu 24.04, kernel 6.8, aarch64 | |
| `docker run --platform linux/amd64 alpine uname -m` | `x86_64` | **x86 containers run** (emulated) |
| `/dev/kvm` inside a container | missing | **No Windows VM inside Docker** on this Mac (rules out dockur/windows here) |

### Route 1: Wine in an x86 container (possible now, small effort)

- Fits the current code: one `apps/<name>/Dockerfile` (`FROM --platform=linux/amd64`, Wine + Xvfb + noVNC)
  plus one `APPS` entry, then `app:notepadpp` opens in a tab.
- **Works well:** Notepad++, 7-Zip, WinSCP, older utilities, many simple tools.
- **Works badly:** Microsoft Office 365, Adobe, apps needing .NET or a newer DirectX, games with anti-cheat.
- **Speed:** x86 emulation plus Wine is slower than native. Fine for small tools.

### Route 2: a real Windows VM (full compatibility, large effort)

- Windows 11 ARM in UTM, VMware Fusion (free for personal use) or Parallels. Windows' built-in x86 emulator
  runs nearly all x86 apps. Needs a Windows license.
- DockBrowser starts and stops the VM and shows each app as its own window via **RDP RemoteApp** (as WinApps
  does), probably through Guacamole. This is DESIGN.md roadmap step 3.
- It can't stay "everything in Docker" on this Mac (no KVM).

**Recommendation:** do the Route 1 spike first. Take on Route 2 only if Wine can't run the apps you care about.

---

## 5. Earlier findings (this session)

### Fork Firefox?

**No.** Firefox isn't Rust-based: it's mostly C++ and JavaScript, with Rust in parts (Stylo, WebRender).
A fork means a huge build, rebasing on 4-week releases plus security fixes forever, and rebranding.
It also drops multi-engine support, and the streaming and container work gains nothing from being inside the engine.
If Gecko-only were ever acceptable, a **Firefox extension + native messaging host** (reusing
`container.rs`/`proxy.rs`) gets real tabs and per-tab proxying (`browser.proxy.onRequest` sees `tabId`)
with no fork.

### Is noVNC secure?

- The **pixel-streaming model** is the security gain: page code runs in the container (inside a VM on macOS),
  and only pixels reach the host.
- **Current hole:** `vnc.html` is served *from inside the container*, so a compromised container could put its
  own JS into the host WebKit tab. It's sandboxed and has no IPC, but that breaks the "pixels only" guarantee.
  **Fix:** the host serves the viewer and the container provides only the stream (DESIGN.md roadmap step 2).
- noVNC is the simplest protocol with the smallest attack surface. WebRTC stacks (neko, Selkies) add surface.
  VNC passwords are weak (8 characters), so use a per-session token instead.
- **Not covered by streaming:** sandbox access to the host and LAN (Docker's default network), no stream auth,
  kernel escapes (the VM helps), and what the user types into a hijacked sandbox.

### Fedora in Docker

- You get Fedora userland only: the kernel is the runtime VM's.
- GNOME needs systemd, so it won't work under these restrictions; XFCE does.
- `xfce4-settings` is required, or xfdesktop fails to start.
- `dnf install` at runtime is impossible (non-root, no capabilities). Add packages to the Dockerfile instead.

### Tauri multiwebview (relevant to step 1)

- `Window::add_child` needs the tauri `unstable` feature (tauri 2.12.5).
- **Security:** a capability scoped with `"windows": ["main"]` also matches every **child webview** in that window
  (tauri `ipc/authority.rs`). Scope with `"webviews": ["main"]` instead. Already done.
- `emit_to("main", …)` matches by label, so events don't reach tab webviews, which have no event permission anyway.

### Incident: orphan cleanup vs a second instance

While testing, a second app instance started while the user's instance was running. The first version of
`remove_orphans` removed **all** `dockbrowser`-labelled containers, so it could have killed the other
instance's tabs. Nothing was lost: the old build didn't label its containers. Fixed: containers are labelled
`dockbrowser=<owner pid>`, and only containers whose owner is dead are removed (`container::tests::owner_liveness`).
