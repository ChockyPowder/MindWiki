//! Talking to a mindwiki-server over HTTPS: logging in and reading/writing pages.
//!
//! Plain `http://` addresses are refused. A server with a normal certificate
//! (e.g. Let's Encrypt) is checked the usual way. For a self-signed server the
//! user types the fingerprint the server printed; we download the certificate,
//! check it has exactly that fingerprint, and from then on trust only it.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::Duration;
use ureq::tls::{Certificate, RootCerts, TlsConfig};

/// A saved connection. The password is never stored, only the login token.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Remote {
    pub url: String,
    pub username: String,
    pub token: String,
    /// The pinned certificate of a self-signed server.
    #[serde(default)]
    pub cert_pem: Option<String>,
}

#[derive(Debug)]
pub enum Error {
    /// Not logged in, or the login expired.
    Auth,
    /// The account is read-only.
    Forbidden,
    /// The page changed on the server since we last saw it.
    Conflict,
    Other(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Auth => write!(f, "login expired"),
            Error::Forbidden => write!(f, "your account is read-only"),
            Error::Conflict => write!(f, "the page changed on the server"),
            Error::Other(s) => write!(f, "{s}"),
        }
    }
}

/// Tidies a typed address and insists on https.
pub fn normalize_url(url: &str) -> Result<String, String> {
    let url = url.trim().trim_end_matches('/');
    if url.is_empty() {
        return Err("Enter the server address".into());
    }
    if url.starts_with("http://") {
        return Err("Only https:// servers are supported, so your password and pages stay private".into());
    }
    let url = if url.contains("://") { url.to_string() } else { format!("https://{url}") };
    if !url.starts_with("https://") || url.len() <= "https://".len() {
        return Err("The address should look like https://example.com:8443".into());
    }
    Ok(url)
}

/// Keeps only hex digits, lowercased, so `AB:12…` and `ab12…` compare equal.
fn clean_fingerprint(fp: &str) -> String {
    fp.chars().filter(char::is_ascii_hexdigit).map(|c| c.to_ascii_lowercase()).collect()
}

fn agent(cert_pem: Option<&str>, verify: bool) -> Result<ureq::Agent, String> {
    let mut tls = TlsConfig::builder();
    if !verify {
        tls = tls.disable_verification(true);
    } else if let Some(pem) = cert_pem {
        let cert = Certificate::from_pem(pem.as_bytes()).map_err(|e| format!("bad saved certificate: {e}"))?;
        tls = tls.root_certs(RootCerts::Specific(Arc::new(vec![cert])));
    }
    let config = ureq::Agent::config_builder()
        .https_only(true)
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(20)))
        .tls_config(tls.build())
        .build();
    Ok(ureq::Agent::new_with_config(config))
}

fn net_error(e: ureq::Error) -> String {
    let s = e.to_string();
    if s.to_lowercase().contains("certificate") {
        format!(
            "The server's certificate isn't trusted ({s}). If the server is self-signed, \
             enter the fingerprint it printed when it started."
        )
    } else {
        format!("Could not reach the server: {s}")
    }
}

/// Downloads a self-signed server's certificate and accepts it only if its
/// SHA-256 fingerprint is the one the user was given. Nothing secret is sent
/// on this first, not-yet-trusted connection.
pub fn fetch_pinned_cert(url: &str, fingerprint: &str) -> Result<String, String> {
    let want = clean_fingerprint(fingerprint);
    if want.len() != 64 {
        return Err("The fingerprint should be 64 hex digits (the server prints it when it starts)".into());
    }
    let pem = agent(None, false)?
        .get(format!("{url}/api/cert"))
        .call()
        .map_err(net_error)?
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("Could not read the server's certificate: {e}"))?;
    let cert = Certificate::from_pem(pem.as_bytes()).map_err(|_| "This doesn't look like a mindwiki server")?;
    let got = hex::encode(Sha256::digest(cert.der()));
    if got != want {
        return Err("The fingerprint doesn't match this server's certificate. \
                    Check it with the server's admin; do not connect if it stays wrong."
            .into());
    }
    Ok(pem)
}

#[derive(Deserialize)]
struct ErrorBody {
    error: Option<String>,
}

/// Logs in and returns a saved connection. `fingerprint` is empty for
/// servers with a normal certificate.
pub fn connect(url: &str, username: &str, password: &str, fingerprint: &str) -> Result<Remote, String> {
    let url = normalize_url(url)?;
    if username.trim().is_empty() || password.is_empty() {
        return Err("Enter your username and password".into());
    }
    let cert_pem = if fingerprint.trim().is_empty() { None } else { Some(fetch_pinned_cert(&url, fingerprint)?) };

    #[derive(Deserialize)]
    struct LoginOk {
        token: String,
        username: String,
    }
    let mut resp = agent(cert_pem.as_deref(), true)?
        .post(format!("{url}/api/login"))
        .send_json(serde_json::json!({ "username": username.trim(), "password": password }))
        .map_err(net_error)?;
    if resp.status().as_u16() != 200 {
        let msg = resp.body_mut().read_json::<ErrorBody>().ok().and_then(|b| b.error);
        return Err(msg.unwrap_or_else(|| format!("Login failed (HTTP {})", resp.status().as_u16())));
    }
    let ok: LoginOk = resp.body_mut().read_json().map_err(|_| "This doesn't look like a mindwiki server")?;
    Ok(Remote { url, username: ok.username, token: ok.token, cert_pem })
}

pub struct Client {
    agent: ureq::Agent,
    base: String,
    auth: String,
}

impl Client {
    pub fn new(remote: &Remote) -> Result<Self, String> {
        Ok(Client {
            agent: agent(remote.cert_pem.as_deref(), true)?,
            base: normalize_url(&remote.url)?,
            auth: format!("Bearer {}", remote.token),
        })
    }

    fn check(mut resp: ureq::http::Response<ureq::Body>) -> Result<ureq::http::Response<ureq::Body>, Error> {
        match resp.status().as_u16() {
            200..=299 => Ok(resp),
            401 => Err(Error::Auth),
            403 => Err(Error::Forbidden),
            409 => Err(Error::Conflict),
            code => {
                let msg = resp.body_mut().read_json::<ErrorBody>().ok().and_then(|b| b.error);
                Err(Error::Other(msg.unwrap_or_else(|| format!("server error (HTTP {code})"))))
            }
        }
    }

    fn other(e: ureq::Error) -> Error {
        Error::Other(net_error(e))
    }

    /// Every page as (title, content hash), plus whether we are read-only.
    pub fn list(&self) -> Result<(Vec<(String, String)>, bool), Error> {
        #[derive(Deserialize)]
        struct Item {
            title: String,
            hash: String,
        }
        #[derive(Deserialize)]
        struct List {
            pages: Vec<Item>,
            read_only: bool,
        }
        let resp = self
            .agent
            .get(format!("{}/api/pages", self.base))
            .header("Authorization", &self.auth)
            .call()
            .map_err(Self::other)?;
        let list: List = Self::check(resp)?.body_mut().read_json().map_err(Self::other)?;
        Ok((list.pages.into_iter().map(|i| (i.title, i.hash)).collect(), list.read_only))
    }

    pub fn get(&self, title: &str) -> Result<String, Error> {
        #[derive(Deserialize)]
        struct Page {
            body: String,
        }
        let resp = self
            .agent
            .get(format!("{}/api/page", self.base))
            .query("title", title)
            .header("Authorization", &self.auth)
            .call()
            .map_err(Self::other)?;
        let page: Page = Self::check(resp)?.body_mut().read_json().map_err(Self::other)?;
        Ok(page.body)
    }

    /// Uploads a page. `base` is the hash we last saw on the server (None if
    /// we believe the page is new); the server refuses if it has changed.
    pub fn put(&self, title: &str, body: &str, base: Option<&str>) -> Result<(), Error> {
        let resp = self
            .agent
            .put(format!("{}/api/page", self.base))
            .header("Authorization", &self.auth)
            .send_json(serde_json::json!({ "title": title, "body": body, "base": base }))
            .map_err(Self::other)?;
        Self::check(resp).map(|_| ())
    }

    pub fn delete(&self, title: &str, base: Option<&str>) -> Result<(), Error> {
        let mut req = self
            .agent
            .delete(format!("{}/api/page", self.base))
            .query("title", title)
            .header("Authorization", &self.auth);
        if let Some(b) = base {
            req = req.query("base", b);
        }
        Self::check(req.call().map_err(Self::other)?).map(|_| ())
    }

    /// Tells the server to forget our login token.
    pub fn logout(&self) {
        let _ = self.agent.post(format!("{}/api/logout", self.base)).header("Authorization", &self.auth).send_empty();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(normalize_url(" example.com:8443/ ").unwrap(), "https://example.com:8443");
        assert_eq!(normalize_url("https://a.b").unwrap(), "https://a.b");
        assert!(normalize_url("http://a.b").is_err());
        assert!(normalize_url("ftp://a.b").is_err());
        assert!(normalize_url("").is_err());
    }

    #[test]
    fn fingerprints() {
        assert_eq!(clean_fingerprint("AB:cd 12"), "abcd12");
    }
}
