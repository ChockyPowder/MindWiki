//! mindwiki-server: shares one wiki with logged-in users over HTTPS.
//!
//! * TLS only. With no certificate given, a self-signed one is created and
//!   its fingerprint printed, so clients can check they reach the right server.
//! * Passwords are stored as Argon2 hashes. Logging in returns a random
//!   token; only its SHA-256 is kept on the server.
//! * Every page request needs a valid token. Read-only users cannot write.

pub mod pages;
pub mod users;

use axum::extract::{ConnectInfo, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use axum_server::tls_rustls::RustlsConfig;
use pages::{Pages, WriteError};
use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use users::{User, Users};

const SESSION_SECS: u64 = 30 * 24 * 3600;
const MAX_FAILS: u32 = 5;
const LOCK_SECS: u64 = 60;

const WELCOME: &str = "# Welcome\n\nThis wiki is shared from a mindwiki server. \
Everyone who can log in sees the same pages.\n\nLink pages with `[[Page Name]]` and tag them with `#tags`.\n";

pub struct Options {
    pub data: PathBuf,
    pub addr: SocketAddr,
    /// PEM certificate chain and key. When both are None a self-signed
    /// certificate is created in the data folder.
    pub cert: Option<PathBuf>,
    pub key: Option<PathBuf>,
    /// Names and IP addresses the self-signed certificate is valid for.
    pub hosts: Vec<String>,
}

pub struct Running {
    pub addr: SocketAddr,
    /// SHA-256 of the server certificate, as lowercase hex.
    pub fingerprint: String,
    pub self_signed: bool,
    pub handle: axum_server::Handle<SocketAddr>,
}

#[derive(Serialize, Deserialize, Clone)]
struct Session {
    user: String,
    expires: u64,
}

struct Throttle {
    fails: u32,
    locked_until: Option<Instant>,
}

struct App {
    users: Users,
    pages: Mutex<Pages>,
    sessions: Mutex<HashMap<String, Session>>,
    sessions_path: PathBuf,
    throttle: Mutex<HashMap<String, Throttle>>,
    cert_pem: String,
    /// Verified against when the username is unknown, so that takes as long
    /// as a real check and doesn't reveal which names exist.
    dummy_hash: String,
}

type Shared = Arc<App>;

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn sha_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Writes a file only its owner can read (used for keys, users and sessions).
pub fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let tmp = path.with_extension("tmp");
    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

/// SHA-256 fingerprint of the first certificate in a PEM file.
pub fn fingerprint_of_pem(pem: &[u8]) -> io::Result<String> {
    let der = CertificateDer::from_pem_slice(pem).map_err(|e| io::Error::other(format!("bad certificate: {e}")))?;
    Ok(sha_hex(&der))
}

/// `ab12…` -> `AB:12:…`, easier to read out and compare.
pub fn pretty_fingerprint(hex: &str) -> String {
    hex.to_uppercase().as_bytes().chunks(2).map(|c| String::from_utf8_lossy(c).into_owned()).collect::<Vec<_>>().join(":")
}

fn load_tls(opts: &Options) -> io::Result<(Vec<u8>, Vec<u8>, bool)> {
    match (&opts.cert, &opts.key) {
        (Some(c), Some(k)) => Ok((std::fs::read(c)?, std::fs::read(k)?, false)),
        (None, None) => {
            let cert_path = opts.data.join("cert.pem");
            let key_path = opts.data.join("key.pem");
            if !cert_path.exists() || !key_path.exists() {
                let mut hosts = opts.hosts.clone();
                for h in ["localhost", "127.0.0.1", "::1"] {
                    if !hosts.iter().any(|x| x == h) {
                        hosts.push(h.to_string());
                    }
                }
                let ck = rcgen::generate_simple_self_signed(hosts)
                    .map_err(|e| io::Error::other(format!("could not create certificate: {e}")))?;
                write_private(&key_path, ck.signing_key.serialize_pem().as_bytes())?;
                std::fs::write(&cert_path, ck.cert.pem())?;
            }
            Ok((std::fs::read(cert_path)?, std::fs::read(key_path)?, true))
        }
        _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "give both --cert and --key, or neither")),
    }
}

/// Starts the server. It runs until `Running::handle` is shut down.
pub async fn start(opts: Options) -> io::Result<(Running, tokio::task::JoinHandle<io::Result<()>>)> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    std::fs::create_dir_all(&opts.data)?;
    let (cert, key, self_signed) = load_tls(&opts)?;
    let fingerprint = fingerprint_of_pem(&cert)?;
    let tls = RustlsConfig::from_pem(cert.clone(), key).await?;

    let mut pages = Pages::open(&opts.data.join("wiki"))?;
    if pages.is_empty() {
        let _ = pages.write("Welcome", WELCOME, None);
    }
    let sessions_path = opts.data.join("sessions.json");
    let mut sessions: HashMap<String, Session> = std::fs::read_to_string(&sessions_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let t = now();
    sessions.retain(|_, s| s.expires > t);

    let app = Arc::new(App {
        users: Users::new(&opts.data),
        pages: Mutex::new(pages),
        sessions: Mutex::new(sessions),
        sessions_path,
        throttle: Mutex::new(HashMap::new()),
        cert_pem: String::from_utf8_lossy(&cert).into_owned(),
        dummy_hash: users::hash_password("not a real password")?,
    });

    let router = Router::new()
        .route("/api/info", get(info))
        .route("/api/cert", get(cert_pem))
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .route("/api/pages", get(list_pages))
        .route("/api/page", get(get_page).put(put_page).delete(delete_page))
        .with_state(app);

    let handle = axum_server::Handle::new();
    let server = axum_server::bind_rustls(opts.addr, tls)
        .handle(handle.clone())
        .serve(router.into_make_service_with_connect_info::<SocketAddr>());
    let task = tokio::spawn(server);
    let addr = handle
        .listening()
        .await
        .ok_or_else(|| io::Error::other("server could not start listening (is the port in use?)"))?;
    Ok((Running { addr, fingerprint, self_signed, handle }, task))
}

// ---------- helpers ----------

fn err(status: StatusCode, msg: &str) -> Response {
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
}

fn save_sessions(app: &App, sessions: &HashMap<String, Session>) {
    if let Ok(json) = serde_json::to_vec(sessions) {
        let _ = write_private(&app.sessions_path, &json);
    }
}

/// Checks the `Authorization: Bearer <token>` header and returns the user.
/// The account is looked up again each time, so removing a user or making
/// them read-only takes effect immediately.
#[allow(clippy::result_large_err)]
fn auth(app: &App, headers: &HeaderMap) -> Result<User, Response> {
    let denied = || err(StatusCode::UNAUTHORIZED, "please log in");
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(denied)?;
    let key = sha_hex(token.as_bytes());
    let name = {
        let mut sessions = app.sessions.lock().unwrap();
        match sessions.get(&key) {
            Some(s) if s.expires > now() => s.user.clone(),
            Some(_) => {
                sessions.remove(&key);
                return Err(denied());
            }
            None => return Err(denied()),
        }
    };
    match app.users.get(&name) {
        Ok(Some(u)) => Ok(u),
        Ok(None) => Err(denied()),
        Err(_) => Err(err(StatusCode::INTERNAL_SERVER_ERROR, "could not read accounts")),
    }
}

/// Returns how many seconds a key (user or address) is still locked out.
fn locked(app: &App, key: &str) -> Option<u64> {
    let mut map = app.throttle.lock().unwrap();
    let t = map.get_mut(key)?;
    match t.locked_until {
        Some(until) if until > Instant::now() => Some((until - Instant::now()).as_secs() + 1),
        Some(_) => {
            map.remove(key);
            None
        }
        None => None,
    }
}

fn record_fail(app: &App, key: &str) {
    let mut map = app.throttle.lock().unwrap();
    if map.len() > 10_000 {
        let now = Instant::now();
        map.retain(|_, t| t.locked_until.is_some_and(|u| u > now));
    }
    let t = map.entry(key.to_string()).or_insert(Throttle { fails: 0, locked_until: None });
    t.fails += 1;
    if t.fails >= MAX_FAILS {
        t.fails = 0;
        t.locked_until = Some(Instant::now() + Duration::from_secs(LOCK_SECS));
    }
}

// ---------- handlers ----------

async fn info() -> Response {
    Json(serde_json::json!({ "server": "mindwiki", "version": env!("CARGO_PKG_VERSION") })).into_response()
}

/// The public certificate. Clients compare its fingerprint with the one the
/// admin gave them before trusting a self-signed server.
async fn cert_pem(State(app): State<Shared>) -> Response {
    app.cert_pem.clone().into_response()
}

#[derive(Deserialize)]
struct LoginReq {
    username: String,
    password: String,
}

async fn login(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(req): Json<LoginReq>,
) -> Response {
    let user_key = format!("user:{}", req.username.to_lowercase());
    let ip: IpAddr = peer.ip();
    let ip_key = format!("ip:{ip}");
    if let Some(secs) = locked(&app, &user_key).or_else(|| locked(&app, &ip_key)) {
        return err(StatusCode::TOO_MANY_REQUESTS, &format!("too many failed logins; try again in {secs}s"));
    }

    let user = match app.users.get(&req.username) {
        Ok(u) => u,
        Err(_) => return err(StatusCode::INTERNAL_SERVER_ERROR, "could not read accounts"),
    };
    let hash = user.as_ref().map(|u| u.hash.clone()).unwrap_or_else(|| app.dummy_hash.clone());
    let password = req.password;
    // Argon2 is deliberately slow, so keep it off the async threads.
    let ok = tokio::task::spawn_blocking(move || users::verify_password(&password, &hash)).await.unwrap_or(false);
    let Some(user) = user.filter(|_| ok) else {
        record_fail(&app, &user_key);
        record_fail(&app, &ip_key);
        return err(StatusCode::UNAUTHORIZED, "wrong username or password");
    };
    app.throttle.lock().unwrap().remove(&user_key);

    let token = hex::encode(rand::random::<[u8; 32]>());
    {
        let mut sessions = app.sessions.lock().unwrap();
        let t = now();
        sessions.retain(|_, s| s.expires > t);
        sessions.insert(sha_hex(token.as_bytes()), Session { user: user.name.clone(), expires: t + SESSION_SECS });
        save_sessions(&app, &sessions);
    }
    Json(serde_json::json!({ "token": token, "username": user.name, "read_only": user.read_only })).into_response()
}

async fn logout(State(app): State<Shared>, headers: HeaderMap) -> Response {
    if let Some(token) =
        headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer "))
    {
        let mut sessions = app.sessions.lock().unwrap();
        if sessions.remove(&sha_hex(token.as_bytes())).is_some() {
            save_sessions(&app, &sessions);
        }
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn list_pages(State(app): State<Shared>, headers: HeaderMap) -> Response {
    let user = match auth(&app, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let pages: Vec<_> = app
        .pages
        .lock()
        .unwrap()
        .list()
        .into_iter()
        .map(|(title, hash)| serde_json::json!({ "title": title, "hash": hash }))
        .collect();
    Json(serde_json::json!({ "pages": pages, "read_only": user.read_only })).into_response()
}

#[derive(Deserialize)]
struct PageQuery {
    title: String,
    base: Option<String>,
}

async fn get_page(State(app): State<Shared>, headers: HeaderMap, Query(q): Query<PageQuery>) -> Response {
    if let Err(r) = auth(&app, &headers) {
        return r;
    }
    match app.pages.lock().unwrap().read(&q.title) {
        Some((body, hash)) => Json(serde_json::json!({ "title": q.title, "body": body, "hash": hash })).into_response(),
        None => err(StatusCode::NOT_FOUND, "no such page"),
    }
}

#[derive(Deserialize)]
struct PutReq {
    title: String,
    body: String,
    base: Option<String>,
}

fn write_error(e: WriteError) -> Response {
    match e {
        WriteError::BadTitle => err(StatusCode::BAD_REQUEST, "invalid page title"),
        WriteError::Conflict(hash) => {
            (StatusCode::CONFLICT, Json(serde_json::json!({ "error": "page changed on the server", "hash": hash })))
                .into_response()
        }
        WriteError::CaseClash(other) => {
            err(StatusCode::BAD_REQUEST, &format!("a page named “{other}” already exists"))
        }
        WriteError::Io(_) => err(StatusCode::INTERNAL_SERVER_ERROR, "could not save the page"),
    }
}

async fn put_page(State(app): State<Shared>, headers: HeaderMap, Json(req): Json<PutReq>) -> Response {
    let user = match auth(&app, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    if user.read_only {
        return err(StatusCode::FORBIDDEN, "your account is read-only");
    }
    match app.pages.lock().unwrap().write(&req.title, &req.body, req.base.as_deref()) {
        Ok(hash) => Json(serde_json::json!({ "hash": hash })).into_response(),
        Err(e) => write_error(e),
    }
}

async fn delete_page(State(app): State<Shared>, headers: HeaderMap, Query(q): Query<PageQuery>) -> Response {
    let user = match auth(&app, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    if user.read_only {
        return err(StatusCode::FORBIDDEN, "your account is read-only");
    }
    match app.pages.lock().unwrap().delete(&q.title, q.base.as_deref()) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => write_error(e),
    }
}
