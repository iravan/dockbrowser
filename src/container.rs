//! Container Runtime Adapter (CRA).

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

// ponytail: drives the podman/docker CLI instead of the socket API; both speak the same flags.
// Move to a socket client (bollard) when we need streaming build logs or events.
fn runtime() -> Result<&'static str, String> {
    static RT: OnceLock<Option<&'static str>> = OnceLock::new();
    RT.get_or_init(|| {
        ["podman", "docker"]
            .into_iter()
            .find(|rt| Command::new(rt).arg("--version").output().is_ok_and(|o| o.status.success()))
    })
    .ok_or_else(|| "no container runtime found (install podman or docker)".into())
}

fn run(args: &[&str]) -> Result<String, String> {
    let out = Command::new(runtime()?).args(args).output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub struct Spec {
    /// OCI image ref, or a local directory containing a Dockerfile.
    pub source: String,
    pub port: u16,
    pub cpus: String,
    pub memory: String,
    /// `KEY=value` pairs.
    pub env: Vec<String>,
}

pub struct Container {
    pub id: String,
    pub host_port: u16,
}

pub fn spawn(spec: &Spec) -> Result<Container, String> {
    if spec.source.starts_with('-') {
        return Err("invalid image".into());
    }
    let image = if Path::new(&spec.source).join("Dockerfile").is_file() {
        run(&["build", "-q", &spec.source])?
    } else {
        spec.source.clone()
    };
    let publish = format!("127.0.0.1::{}", spec.port);
    let cpus = format!("--cpus={}", spec.cpus);
    let memory = format!("--memory={}", spec.memory);
    let owner = format!("--label=dockbrowser={}", std::process::id());
    let mut args = vec![
        "run", "-d", "--rm",
        "-p", &publish,
        &owner,
        "--cap-drop=ALL",
        "--security-opt=no-new-privileges",
        "--shm-size=256m", // the 64m default crashes browser tabs; still capped by --memory
        &cpus,
        &memory,
    ];
    for kv in &spec.env {
        args.extend(["-e", kv]);
    }
    args.push(&image);
    let id = run(&args)?;
    let mapped = run(&["port", &id, &spec.port.to_string()]);
    let host_port = mapped
        .as_deref()
        .ok()
        .and_then(|m| m.lines().next()?.rsplit(':').next()?.parse().ok());
    let Some(host_port) = host_port else {
        stop(&id);
        return Err(format!("container exposed no port {}: {:?}", spec.port, mapped));
    };
    wait_ready(host_port);
    Ok(Container { id, host_port })
}

/// Tabs talk HTTP to their backend, so ready = it answers an HTTP request.
/// (A bare TCP connect isn't enough: VM port-forwarders accept before the app listens.)
fn wait_ready(port: u16) {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let deadline = Instant::now() + Duration::from_secs(15);
    let answers = || -> std::io::Result<bool> {
        let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(300))?;
        s.set_read_timeout(Some(Duration::from_secs(1)))?;
        s.write_all(b"HEAD / HTTP/1.0\r\n\r\n")?;
        Ok(s.read(&mut [0u8; 1])? == 1)
    };
    while Instant::now() < deadline && !answers().unwrap_or(false) {
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// SIGTERM, then SIGKILL after 5s. `--rm` removes the container afterwards.
pub fn stop(id: &str) {
    let _ = run(&["stop", "-t", "5", id]);
}

/// Removes containers whose owning app process is gone (crash, kill -9). Other running instances keep theirs.
pub fn remove_orphans() {
    let Ok(ids) = run(&["ps", "-aq", "--filter", "label=dockbrowser"]) else { return };
    let orphans: Vec<&str> = ids
        .split_whitespace()
        .filter(|id| {
            let owner = run(&["inspect", "--format", "{{index .Config.Labels \"dockbrowser\"}}", id]).unwrap_or_default();
            !owner.parse().is_ok_and(pid_alive)
        })
        .collect();
    if !orphans.is_empty() {
        let _ = run(&[&["rm", "-f"][..], &orphans].concat());
    }
}

fn pid_alive(pid: u32) -> bool {
    Command::new("kill").args(["-0", &pid.to_string()]).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};

    #[test]
    fn owner_liveness() {
        assert!(super::pid_alive(std::process::id()));
        assert!(!super::pid_alive(999_999_999));
    }

    /// Needs a running podman/docker: `cargo test -- --ignored`
    #[test]
    #[ignore]
    fn dockerfile_spawn_serve_stop() {
        let dir = std::env::temp_dir().join(format!("dockbrowser-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Dockerfile"), "FROM python:3-alpine\nCMD [\"python\", \"-m\", \"http.server\", \"8000\"]\n").unwrap();
        let c = super::spawn(&super::Spec {
            source: dir.to_string_lossy().into(),
            port: 8000,
            cpus: "1".into(),
            memory: "128m".into(),
            env: vec![],
        })
        .unwrap();
        let mut s = std::net::TcpStream::connect(("127.0.0.1", c.host_port)).unwrap();
        s.write_all(b"GET / HTTP/1.0\r\n\r\n").unwrap();
        let mut body = String::new();
        s.read_to_string(&mut body).unwrap();
        super::stop(&c.id);
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(body.starts_with("HTTP/1.0 200"), "{body}");
        assert!(super::run(&["inspect", &c.id]).is_err(), "container not removed");
    }

    /// The bundled Firefox image serves noVNC and opens the URL from the env.
    #[test]
    #[ignore]
    fn browser_container() {
        let c = super::spawn(&super::Spec {
            source: concat!(env!("CARGO_MANIFEST_DIR"), "/apps/firefox").into(),
            port: 6080,
            cpus: "1".into(),
            memory: "2g".into(),
            env: vec!["DOCKBROWSER_URL=https://example.com/".into()],
        })
        .unwrap();
        let mut s = std::net::TcpStream::connect(("127.0.0.1", c.host_port)).unwrap();
        s.write_all(b"GET /vnc.html HTTP/1.0\r\n\r\n").unwrap();
        let mut body = String::new();
        s.read_to_string(&mut body).unwrap();
        let procs = super::run(&["top", &c.id, "-o", "pid,args"]);
        super::stop(&c.id);
        assert!(body.starts_with("HTTP/1.0 200") || body.starts_with("HTTP/1.1 200"), "{body:.200}");
        assert!(procs.unwrap().contains("firefox-esr --width 1280 --height 800 https://example.com/"));
    }
}
