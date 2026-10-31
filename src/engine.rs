//! Render Engine Abstraction Layer (REAL).

use std::io::ErrorKind;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

use tauri::webview::PageLoadEvent;
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Rect, Url, Webview, WebviewBuilder, WebviewUrl, Window};

pub trait RenderEngine: Send + Sync {
    fn id(&self) -> &'static str;
    fn open(&self, app: &AppHandle, tab: u32, url: &Url) -> Result<Box<dyn EngineTab>, String>;
}

pub trait EngineTab: Send {
    fn is_alive(&mut self) -> bool;
    fn close(&mut self);
    /// The in-window webview, if this tab renders inside the shell window.
    fn webview(&self) -> Option<&Webview> {
        None
    }
}

/// Tabs fill the window below the shell's toolbar. `#toolbar` in ui/index.html must be exactly this tall.
pub const TOOLBAR_HEIGHT: f64 = 84.0;

pub fn content_bounds(window: &Window) -> Result<Rect, String> {
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let size = window.inner_size().map_err(|e| e.to_string())?.to_logical::<f64>(scale);
    Ok(Rect {
        position: LogicalPosition::new(0.0, TOOLBAR_HEIGHT).into(),
        size: LogicalSize::new(size.width, (size.height - TOOLBAR_HEIGHT).max(0.0)).into(),
    })
}

pub const ENGINES: &[&dyn RenderEngine] = &[
    &WebKit,
    &External {
        id: "blink",
        bins: &[
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
            "google-chrome",
            "chromium",
        ],
        profile_files: &[],
        args: |profile, url| {
            vec![
                format!("--user-data-dir={profile}"),
                "--no-first-run".into(),
                "--no-default-browser-check".into(),
                url.into(),
            ]
        },
    },
    &External {
        id: "gecko",
        bins: &["/Applications/Firefox.app/Contents/MacOS/firefox", "firefox"],
        // trust the OS keychain, where our local CA lives
        profile_files: &[("user.js", "user_pref(\"security.enterprise_roots.enabled\", true);\n")],
        args: |profile, url| {
            vec!["--new-instance".into(), "--profile".into(), profile.into(), url.into()]
        },
    },
];

pub fn find(id: &str) -> Option<&'static dyn RenderEngine> {
    ENGINES.iter().copied().find(|e| e.id() == id)
}

/// WebKit via the platform webview (WKWebView on macOS, WebKitGTK on Linux), as a child of the shell window.
struct WebKit;

impl RenderEngine for WebKit {
    fn id(&self) -> &'static str {
        "webkit"
    }

    fn open(&self, app: &AppHandle, tab: u32, url: &Url) -> Result<Box<dyn EngineTab>, String> {
        // fresh label per open: a destroyed window's label lingers briefly, which breaks engine switching
        static OPENS: AtomicU32 = AtomicU32::new(0);
        let label = format!("tab-{tab}-{}", OPENS.fetch_add(1, Ordering::Relaxed));
        let window = app.get_window("main").ok_or("no shell window")?;
        let bounds = content_bounds(&window)?;
        let builder = WebviewBuilder::new(label, WebviewUrl::External(url.clone()))
            .incognito(true) // ephemeral data store per tab
            .on_page_load(move |webview, page| {
                if page.event() == PageLoadEvent::Finished {
                    let _ = webview.emit_to("main", "tab-url", (tab, page.url().as_str()));
                }
            });
        let webview = window.add_child(builder, bounds.position, bounds.size).map_err(|e| e.to_string())?;
        Ok(Box::new(WebviewTab { webview }))
    }
}

struct WebviewTab {
    webview: Webview,
}

impl EngineTab for WebviewTab {
    fn is_alive(&mut self) -> bool {
        self.webview.app_handle().get_webview(self.webview.label()).is_some()
    }

    fn close(&mut self) {
        let _ = self.webview.close();
    }

    fn webview(&self) -> Option<&Webview> {
        Some(&self.webview)
    }
}

// ponytail: Blink/Gecko run as separate browser processes with a throwaway profile,
// not embedded in our window. Swap for CEF / Gecko embedding when in-window tabs are required.
struct External {
    id: &'static str,
    bins: &'static [&'static str],
    profile_files: &'static [(&'static str, &'static str)],
    args: fn(&str, &str) -> Vec<String>,
}

impl RenderEngine for External {
    fn id(&self) -> &'static str {
        self.id
    }

    fn open(&self, _: &AppHandle, tab: u32, url: &Url) -> Result<Box<dyn EngineTab>, String> {
        let profile = std::env::temp_dir().join(format!("dockbrowser-{}-{tab}-{}", std::process::id(), self.id));
        std::fs::create_dir_all(&profile).map_err(|e| e.to_string())?;
        for (name, contents) in self.profile_files {
            std::fs::write(profile.join(name), contents).map_err(|e| e.to_string())?;
        }
        let args = (self.args)(&profile.to_string_lossy(), url.as_str());
        for bin in self.bins {
            match Command::new(bin).args(&args).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
                Ok(child) => return Ok(Box::new(ProcessTab { child, profile })),
                Err(e) if e.kind() == ErrorKind::NotFound => continue,
                Err(e) => return Err(e.to_string()),
            }
        }
        let _ = std::fs::remove_dir_all(&profile);
        Err(format!("{} engine not installed (tried {:?})", self.id, self.bins))
    }
}

struct ProcessTab {
    child: Child,
    profile: PathBuf,
}

impl EngineTab for ProcessTab {
    fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    fn close(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.profile);
    }
}
