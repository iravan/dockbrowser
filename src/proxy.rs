//! Internal TLS reverse proxy: https://tab-<id>.dockbrowser.localhost:<port> -> that tab's container.
//! `*.localhost` resolves to loopback in every engine, so no DNS server is needed.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, GeneralSubtree, IsCa, Issuer,
    KeyPair, KeyUsagePurpose, NameConstraints,
};
use time::{Duration, OffsetDateTime};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::rustls::{self, pki_types::PrivateKeyDer};
use tokio_rustls::TlsAcceptor;

pub const DOMAIN: &str = "dockbrowser.localhost";
const CA_NAME: &str = "DockBrowser Local CA";

pub struct Proxy {
    pub port: u16,
    routes: Arc<Mutex<HashMap<u32, u16>>>,
}

impl Proxy {
    /// Loads (or creates) the CA in `dir`, issues a fresh leaf cert for this launch, starts listening.
    pub fn start(dir: &Path) -> Result<Proxy, String> {
        let issuer = load_or_create_ca(dir)?;
        let key = KeyPair::generate().map_err(|e| e.to_string())?;
        let mut leaf = CertificateParams::new(vec![format!("*.{DOMAIN}")]).map_err(|e| e.to_string())?;
        leaf.distinguished_name.push(DnType::CommonName, format!("*.{DOMAIN}"));
        leaf.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        leaf.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        leaf.not_before = OffsetDateTime::now_utc() - Duration::days(1);
        leaf.not_after = OffsetDateTime::now_utc() + Duration::days(30);
        let cert = leaf.signed_by(&key, &issuer).map_err(|e| e.to_string())?;

        // No ALPN = HTTP/1.1 only, so browsers can't coalesce two tabs' hosts onto one h2 connection.
        let config = rustls::ServerConfig::builder_with_provider(rustls::crypto::ring::default_provider().into())
            .with_safe_default_protocol_versions()
            .map_err(|e| e.to_string())?
            .with_no_client_auth()
            .with_single_cert(vec![cert.der().clone()], PrivateKeyDer::Pkcs8(key.serialize_der().into()))
            .map_err(|e| e.to_string())?;
        let acceptor = TlsAcceptor::from(Arc::new(config));

        let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let routes = Arc::new(Mutex::new(HashMap::new()));
        tauri::async_runtime::spawn(serve(listener, acceptor, routes.clone()));
        Ok(Proxy { port, routes })
    }

    pub fn route(&self, tab: u32, backend_port: u16) -> String {
        self.routes.lock().unwrap().insert(tab, backend_port);
        format!("https://tab-{tab}.{DOMAIN}:{}/", self.port)
    }

    pub fn unroute(&self, tab: u32) {
        self.routes.lock().unwrap().remove(&tab);
    }
}

async fn serve(listener: std::net::TcpListener, acceptor: TlsAcceptor, routes: Arc<Mutex<HashMap<u32, u16>>>) {
    let listener = TcpListener::from_std(listener).expect("proxy listener");
    loop {
        let Ok((tcp, _)) = listener.accept().await else { continue };
        let (acceptor, routes) = (acceptor.clone(), routes.clone());
        tauri::async_runtime::spawn(async move {
            let Ok(mut tls) = acceptor.accept(tcp).await else { return };
            let backend = tls
                .get_ref()
                .1
                .server_name()
                .and_then(tab_id)
                .and_then(|id| routes.lock().unwrap().get(&id).copied());
            let Some(port) = backend else { return };
            let Ok(mut upstream) = TcpStream::connect(("127.0.0.1", port)).await else { return };
            let _ = tokio::io::copy_bidirectional(&mut tls, &mut upstream).await;
        });
    }
}

fn tab_id(host: &str) -> Option<u32> {
    host.strip_suffix(DOMAIN)?.strip_suffix('.')?.strip_prefix("tab-")?.parse().ok()
}

pub fn ca_path(dir: &Path) -> PathBuf {
    dir.join("ca.pem")
}

fn load_or_create_ca(dir: &Path) -> Result<Issuer<'static, KeyPair>, String> {
    let (cert_path, key_path) = (ca_path(dir), dir.join("ca.key"));
    if let (Ok(cert), Ok(key)) = (std::fs::read_to_string(&cert_path), std::fs::read_to_string(&key_path)) {
        let key = KeyPair::from_pem(&key).map_err(|e| e.to_string())?;
        return Issuer::from_ca_cert_pem(&cert, key).map_err(|e| e.to_string());
    }
    let key = KeyPair::generate().map_err(|e| e.to_string())?;
    let mut ca = CertificateParams::default();
    ca.distinguished_name.push(DnType::CommonName, CA_NAME);
    ca.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    ca.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    // Even if the key leaks, this CA can't vouch for anything outside our domain.
    ca.name_constraints = Some(NameConstraints {
        permitted_subtrees: vec![GeneralSubtree::DnsName(DOMAIN.into())],
        excluded_subtrees: vec![],
    });
    ca.not_before = OffsetDateTime::now_utc() - Duration::days(1);
    ca.not_after = OffsetDateTime::now_utc() + Duration::days(3650);
    let cert = ca.self_signed(&key).map_err(|e| e.to_string())?;

    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    write_private(&key_path, &key.serialize_pem())?;
    std::fs::write(&cert_path, cert.pem()).map_err(|e| e.to_string())?;
    Ok(Issuer::new(ca, key))
}

fn write_private(path: &Path, contents: &str) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .and_then(|mut f| f.write_all(contents.as_bytes()))
        .map_err(|e| e.to_string())
}

/// Adds the CA to the login keychain as a trusted root if it isn't already (macOS asks for the password once).
/// WebKit and Chrome read the keychain; Firefox does via `security.enterprise_roots.enabled` in its profile.
// ponytail: macOS only. Linux needs per-store installs (NSS certutil, update-ca-certificates).
pub fn ensure_trusted(dir: &Path) -> Result<(), String> {
    let ca = ca_path(dir);
    let ca = ca.to_str().ok_or("non-utf8 path")?;
    let security = |args: &[&str]| std::process::Command::new("security").args(args).output();
    if security(&["verify-cert", "-c", ca]).is_ok_and(|o| o.status.success()) {
        return Ok(());
    }
    let keychain = format!("{}/Library/Keychains/login.keychain-db", std::env::var("HOME").map_err(|e| e.to_string())?);
    let out = security(&["add-trusted-cert", "-r", "trustRoot", "-k", &keychain, ca]).map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!("CA not trusted: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::pki_types::{pem::PemObject, CertificateDer, ServerName};
    use std::io::{Read, Write};

    #[test]
    fn tab_ids() {
        assert_eq!(tab_id("tab-7.dockbrowser.localhost"), Some(7));
        assert_eq!(tab_id("tab-7.evil.localhost"), None);
        assert_eq!(tab_id("xtab-7.dockbrowser.localhost"), None);
        assert_eq!(tab_id("tab-7dockbrowser.localhost"), None);
    }

    /// TLS terminates with a cert that chains to our CA, and SNI picks the right backend.
    #[test]
    fn routes_by_sni() {
        let dir = std::env::temp_dir().join(format!("dockbrowser-proxy-{}", std::process::id()));
        let proxy = Proxy::start(&dir).unwrap();
        Proxy::start(&dir).unwrap(); // second start reuses the stored CA

        let backend = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let backend_port = backend.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut s, _) = backend.accept().unwrap();
            s.read(&mut [0u8; 1024]).unwrap();
            s.write_all(b"HTTP/1.0 200 OK\r\n\r\nhello from tab").unwrap();
        });
        let url = proxy.route(3, backend_port);
        assert_eq!(url, format!("https://tab-3.{DOMAIN}:{}/", proxy.port));

        let mut roots = rustls::RootCertStore::empty();
        roots.add(CertificateDer::from_pem_file(ca_path(&dir)).unwrap()).unwrap();
        let config = rustls::ClientConfig::builder_with_provider(rustls::crypto::ring::default_provider().into())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let name = ServerName::try_from(format!("tab-3.{DOMAIN}")).unwrap();
        let conn = rustls::ClientConnection::new(Arc::new(config), name).unwrap();
        let tcp = std::net::TcpStream::connect(("127.0.0.1", proxy.port)).unwrap();
        let mut tls = rustls::StreamOwned::new(conn, tcp);
        tls.write_all(b"GET / HTTP/1.0\r\n\r\n").unwrap();
        let mut body = Vec::new();
        let _ = tls.read_to_end(&mut body); // keeps what was read even if close_notify is missing
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(body.ends_with(b"hello from tab"), "{}", String::from_utf8_lossy(&body));
    }
}
