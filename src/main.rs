//! UI shell + core orchestrator: tab open -> engine + container spawn, tab close -> teardown.

mod container;
mod engine;
mod proxy;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, RunEvent, Url, Webview, WindowEvent};

use proxy::Proxy;

const DEFAULT_ENGINE: &str = "webkit";
/// Engine choice that opens a web URL in the Firefox app instead of a host engine.
const CONTAINER_ENGINE: &str = "container";

/// Built-in app catalog: `app:<name>` in the address bar.
// ponytail: compiled-in list; JSON manifests fetched over https are DESIGN.md roadmap step 7.
struct App {
    name: &'static str,
    dockerfile: &'static str,
    port: u16,
    viewer: &'static str,
    memory: &'static str,
}

const NOVNC: &str = "vnc.html?autoconnect=1&resize=scale";
const APPS: &[App] = &[
    App { name: "firefox", dockerfile: include_str!("../apps/firefox/Dockerfile"), port: 6080, viewer: NOVNC, memory: "2g" },
    App { name: "fedora", dockerfile: include_str!("../apps/fedora/Dockerfile"), port: 6080, viewer: NOVNC, memory: "2g" },
];

struct Tab {
    id: u32,
    engine: Box<dyn engine::EngineTab>,
    engine_id: &'static str,
    /// What the engine loaded: the site, or the container's viewer behind the proxy.
    url: Url,
    /// What the user typed, shown in the address bar for container tabs.
    address: String,
    title: String,
    container: Option<container::Container>,
}

impl Tab {
    fn info(&self) -> TabInfo {
        TabInfo {
            id: self.id,
            engine: self.engine_id,
            address: if self.container.is_some() { self.address.clone() } else { self.url.to_string() },
            title: self.title.clone(),
            container: self.container.as_ref().map(|c| c.id.chars().take(12).collect()),
        }
    }

    fn teardown(mut self, proxy: &Proxy) {
        self.engine.close();
        if let Some(c) = self.container {
            proxy.unroute(self.id);
            container::stop(&c.id);
        }
    }
}

#[derive(Default)]
struct Tabs {
    next: AtomicU32,
    open: Mutex<HashMap<u32, Tab>>,
}

/// Per-domain engine preference, persisted as JSON. A rule for `example.com` also covers its subdomains.
struct Prefs {
    path: PathBuf,
    domains: Mutex<HashMap<String, String>>,
}

impl Prefs {
    fn load(path: PathBuf) -> Prefs {
        let domains = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Prefs { path, domains: Mutex::new(domains) }
    }

    fn engine_for(&self, host: &str) -> Option<String> {
        let domains = self.domains.lock().unwrap();
        let mut host = host;
        loop {
            if let Some(engine) = domains.get(host) {
                return Some(engine.clone());
            }
            host = host.split_once('.')?.1;
        }
    }

    fn remember(&self, host: &str, engine: &str) -> Result<(), String> {
        let mut domains = self.domains.lock().unwrap();
        domains.insert(host.strip_prefix("www.").unwrap_or(host).to_string(), engine.to_string());
        let json = serde_json::to_vec_pretty(&*domains).map_err(|e| e.to_string())?;
        std::fs::write(&self.path, json).map_err(|e| e.to_string())
    }
}

#[derive(serde::Deserialize)]
struct OpenRequest {
    /// Address bar text: a web URL or `app:<name>`.
    url: String,
    /// An engine id, "container" (web URL in the Firefox app), or "auto" to use the domain preference.
    engine: String,
    #[serde(default)]
    remember: bool,
    /// Custom container: image ref or Dockerfile directory, plus the port it serves HTTP on.
    image: Option<String>,
    port: Option<u16>,
    cpus: Option<String>,
    memory: Option<String>,
}

#[derive(Clone, serde::Serialize)]
struct TabInfo {
    id: u32,
    engine: &'static str,
    address: String,
    title: String,
    container: Option<String>,
}

fn parse_url(input: &str) -> Result<Url, String> {
    let input = input.trim();
    let with_scheme = if input.contains("://") { input.to_string() } else { format!("https://{input}") };
    let url = Url::parse(&with_scheme).map_err(|e| format!("bad url: {e}"))?;
    match url.scheme() {
        "http" | "https" => Ok(url),
        s => Err(format!("scheme {s} not allowed")),
    }
}

/// Only the shell webview may drive tabs and containers; tab webviews hold untrusted content.
fn require_shell(webview: &Webview) -> Result<(), String> {
    (webview.label() == "main").then_some(()).ok_or_else(|| "forbidden".into())
}

/// What an address opens: a site in a host engine, or an app in a container shown in the viewer engine.
struct Target {
    engine: &'static dyn engine::RenderEngine,
    address: String,
    title: String,
    /// The typed web URL, if any: what a host engine loads, and the key for engine prefs.
    site: Option<Url>,
    /// Container to run, and the viewer path the tab loads behind the proxy instead of `site`.
    app: Option<(container::Spec, &'static str)>,
}

fn resolve(app: &AppHandle, prefs: &Prefs, req: &OpenRequest) -> Result<Target, String> {
    let viewer = engine::find(DEFAULT_ENGINE).ok_or("no viewer engine")?;
    let cpus = req.cpus.clone().unwrap_or_else(|| "1".into());
    if let Some(source) = req.image.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let spec = container::Spec {
            source: source.into(),
            port: req.port.ok_or("container port required")?,
            cpus,
            memory: req.memory.clone().unwrap_or_else(|| "512m".into()),
            env: vec![],
        };
        let title = source.trim_end_matches('/').rsplit('/').next().unwrap_or(source).into();
        return Ok(Target { engine: viewer, address: source.into(), title, site: None, app: Some((spec, "")) });
    }
    let (name, site) = match req.url.trim().strip_prefix("app:") {
        Some(name) => (name.trim(), None),
        None => {
            let url = parse_url(&req.url)?;
            let id = match req.engine.as_str() {
                "auto" => url.host_str().and_then(|h| prefs.engine_for(h)).unwrap_or_else(|| DEFAULT_ENGINE.into()),
                id => id.into(),
            };
            if id != CONTAINER_ENGINE {
                let engine = engine::find(&id).ok_or_else(|| format!("unknown engine {id}"))?;
                let title = url.host_str().unwrap_or_default().into();
                return Ok(Target { engine, address: url.to_string(), title, site: Some(url), app: None });
            }
            ("firefox", Some(url))
        }
    };
    let a = APPS.iter().find(|a| a.name == name).ok_or_else(|| format!("unknown app {name}"))?;
    let spec = container::Spec {
        source: app_context(app, a)?,
        port: a.port,
        cpus,
        memory: req.memory.clone().unwrap_or_else(|| a.memory.into()),
        env: site.iter().map(|u| format!("DOCKBROWSER_URL={u}")).collect(),
    };
    Ok(Target {
        engine: viewer,
        address: site.as_ref().map_or_else(|| format!("app:{name}"), Url::to_string),
        title: site.as_ref().and_then(|u| u.host_str()).unwrap_or(a.name).into(),
        site,
        app: Some((spec, a.viewer)),
    })
}

/// Writes an app's bundled Dockerfile to app data (so release builds have it too) and returns that build context.
// ponytail: noVNC has no password; any local process can reach the loopback port. Add a stream token (DESIGN.md §9).
fn app_context(app: &AppHandle, a: &App) -> Result<String, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?.join("apps").join(a.name);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("Dockerfile"), a.dockerfile).map_err(|e| e.to_string())?;
    dir.into_os_string().into_string().map_err(|_| "non-utf8 app data path".into())
}

#[tauri::command]
async fn open_tab(app: AppHandle, webview: Webview, req: OpenRequest) -> Result<TabInfo, String> {
    require_shell(&webview)?;
    tauri::async_runtime::spawn_blocking(move || open_tab_blocking(&app, req))
        .await
        .map_err(|e| e.to_string())?
}

fn open_tab_blocking(app: &AppHandle, req: OpenRequest) -> Result<TabInfo, String> {
    let (tabs, proxy, prefs) = (app.state::<Tabs>(), app.state::<Proxy>(), app.state::<Prefs>());
    let id = tabs.next.fetch_add(1, Ordering::Relaxed);
    let target = resolve(app, &prefs, &req)?;
    let container = match &target.app {
        Some((spec, _)) => Some(container::spawn(spec)?),
        None => None,
    };
    let opened = (|| {
        let url = match (&container, &target.app) {
            (Some(c), Some((_, viewer))) => parse_url(&(proxy.route(id, c.host_port) + viewer))?,
            _ => target.site.clone().ok_or("nothing to open")?,
        };
        Ok::<_, String>((target.engine.open(app, id, &url)?, url))
    })();
    let (engine_tab, url) = match opened {
        Ok(v) => v,
        Err(e) => {
            if let Some(c) = container {
                proxy.unroute(id);
                container::stop(&c.id);
            }
            return Err(e);
        }
    };
    if req.remember && req.engine != "auto" {
        if let Some(host) = target.site.as_ref().and_then(|u| u.host_str()) {
            prefs.remember(host, &req.engine)?;
        }
    }
    let tab = Tab {
        id,
        engine: engine_tab,
        engine_id: target.engine.id(),
        url,
        address: target.address,
        title: target.title,
        container,
    };
    let info = tab.info();
    tabs.open.lock().unwrap().insert(id, tab);
    Ok(info)
}

/// Re-opens a tab in another engine. The container and proxy route stay up.
#[tauri::command]
async fn switch_engine(app: AppHandle, webview: Webview, id: u32, engine: String) -> Result<TabInfo, String> {
    require_shell(&webview)?;
    tauri::async_runtime::spawn_blocking(move || {
        let new = engine::find(&engine).ok_or("unknown engine")?;
        let tabs = app.state::<Tabs>();
        let mut tab = tabs.open.lock().unwrap().remove(&id).ok_or("no such tab")?;
        // carry over where the user browsed to (http/https only, like any navigation URL)
        let current = tab.engine.webview().and_then(|w| w.url().ok()).filter(|u| matches!(u.scheme(), "http" | "https"));
        if let (Some(u), None) = (current, &tab.container) {
            tab.url = u;
        }
        // open the new engine before closing the old one, so a failure leaves the tab as it was
        let opened = if new.id() == tab.engine_id { Err("already on that engine".into()) } else { new.open(&app, id, &tab.url) };
        let result = opened.map(|engine_tab| {
            std::mem::replace(&mut tab.engine, engine_tab).close();
            tab.engine_id = new.id();
            tab.info()
        });
        tabs.open.lock().unwrap().insert(id, tab);
        result
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Shows one tab's in-window webview and hides the rest (`None` = the shell's start page).
#[tauri::command]
async fn select_tab(app: AppHandle, webview: Webview, id: Option<u32>) -> Result<(), String> {
    require_shell(&webview)?;
    for tab in app.state::<Tabs>().open.lock().unwrap().values() {
        if let Some(w) = tab.engine.webview() {
            let _ = if Some(tab.id) == id { w.show().and_then(|_| w.set_focus()) } else { w.hide() };
        }
    }
    Ok(())
}

#[tauri::command]
async fn navigate(app: AppHandle, webview: Webview, id: u32, url: String) -> Result<(), String> {
    require_shell(&webview)?;
    let url = parse_url(&url)?;
    let tabs = app.state::<Tabs>();
    let open = tabs.open.lock().unwrap();
    let tab = open.get(&id).filter(|t| t.container.is_none()).ok_or("not a web tab")?;
    tab.engine.webview().ok_or("tab runs in an external window")?.navigate(url).map_err(|e| e.to_string())
}

/// Back / forward / reload in an in-window tab.
#[tauri::command]
async fn history(app: AppHandle, webview: Webview, id: u32, action: String) -> Result<(), String> {
    require_shell(&webview)?;
    let js = match action.as_str() {
        "back" => "history.back()",
        "forward" => "history.forward()",
        "reload" => "location.reload()",
        _ => return Err("unknown action".into()),
    };
    let tabs = app.state::<Tabs>();
    let open = tabs.open.lock().unwrap();
    let w = open.get(&id).and_then(|t| t.engine.webview()).ok_or("tab runs in an external window")?;
    w.eval(js).map_err(|e| e.to_string())
}

#[tauri::command]
async fn close_tab(app: AppHandle, webview: Webview, id: u32) -> Result<(), String> {
    require_shell(&webview)?;
    let tab = app.state::<Tabs>().open.lock().unwrap().remove(&id);
    if let Some(tab) = tab {
        tauri::async_runtime::spawn_blocking(move || tab.teardown(&app.state::<Proxy>()));
    }
    Ok(())
}

#[derive(serde::Serialize)]
struct Catalog {
    engines: Vec<&'static str>,
    apps: Vec<&'static str>,
}

#[tauri::command]
fn catalog(webview: Webview) -> Result<Catalog, String> {
    require_shell(&webview)?;
    let mut engines: Vec<_> = engine::ENGINES.iter().map(|e| e.id()).collect();
    engines.push(CONTAINER_ENGINE);
    Ok(Catalog { engines, apps: APPS.iter().map(|a| a.name).collect() })
}

// ponytail: 1s poll for tabs the user closed from the engine side; event hooks per engine if latency matters.
fn reap_closed_tabs(app: AppHandle) {
    loop {
        std::thread::sleep(Duration::from_secs(1));
        let dead: Vec<(u32, Tab)> = {
            let mut open = app.state::<Tabs>().inner().open.lock().unwrap();
            let ids: Vec<u32> = open.iter_mut().filter_map(|(id, t)| (!t.engine.is_alive()).then_some(*id)).collect();
            ids.into_iter().filter_map(|id| open.remove_entry(&id)).collect()
        };
        for (id, tab) in dead {
            tab.teardown(&app.state::<Proxy>());
            let _ = app.emit_to("main", "tab-closed", id);
        }
    }
}

fn main() {
    tauri::Builder::default()
        .manage(Tabs::default())
        .invoke_handler(tauri::generate_handler![
            open_tab, switch_engine, close_tab, select_tab, navigate, history, catalog
        ])
        .on_window_event(|window, event| {
            if matches!(event, WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }) {
                if let Ok(bounds) = engine::content_bounds(window) {
                    for w in window.webviews().into_iter().filter(|w| w.label() != "main") {
                        let _ = w.set_bounds(bounds);
                    }
                }
            }
        })
        .setup(|app| {
            container::remove_orphans();
            let dir = app.path().app_data_dir()?;
            app.manage(Proxy::start(&dir)?);
            if let Err(e) = proxy::ensure_trusted(&dir) {
                eprintln!("warning: {e}. Container tabs will show certificate errors.");
            }
            app.manage(Prefs::load(dir.join("prefs.json")));
            let handle = app.handle().clone();
            std::thread::spawn(move || reap_closed_tabs(handle));
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build app")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                let all: Vec<Tab> = app.state::<Tabs>().open.lock().unwrap().drain().map(|(_, t)| t).collect();
                all.into_iter().for_each(|t| t.teardown(&app.state::<Proxy>()));
            }
        });
}

#[cfg(test)]
mod tests {
    use super::{parse_url, Prefs};

    #[test]
    fn url_parsing() {
        assert_eq!(parse_url("example.com").unwrap().as_str(), "https://example.com/");
        assert_eq!(parse_url("http://127.0.0.1:8000").unwrap().as_str(), "http://127.0.0.1:8000/");
        assert!(parse_url("file:///etc/passwd").is_err());
        assert!(parse_url("javascript:alert(1)").is_err());
        // never hands a leading-dash string (a browser flag) to an engine process
        assert!(parse_url("--remote-debugging-port=9222").map_or(true, |u| u.as_str().starts_with("https://")));
    }

    #[test]
    fn domain_prefs() {
        let path = std::env::temp_dir().join(format!("dockbrowser-prefs-{}.json", std::process::id()));
        let prefs = Prefs::load(path.clone());
        prefs.remember("www.example.com", "gecko").unwrap();
        prefs.remember("docs.example.com", "blink").unwrap();
        let prefs = Prefs::load(path.clone()); // survives reload
        std::fs::remove_file(&path).unwrap();
        assert_eq!(prefs.engine_for("example.com").as_deref(), Some("gecko"));
        assert_eq!(prefs.engine_for("app.example.com").as_deref(), Some("gecko"));
        assert_eq!(prefs.engine_for("a.docs.example.com").as_deref(), Some("blink"));
        assert_eq!(prefs.engine_for("example.org"), None);
    }
}
