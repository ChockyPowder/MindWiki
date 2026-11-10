//! User accounts, stored in `users.json` with Argon2 password hashes.

use argon2::Argon2;
use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct User {
    pub name: String,
    /// Argon2 hash in PHC string format. The password itself is never stored.
    pub hash: String,
    #[serde(default)]
    pub read_only: bool,
}

pub struct Users {
    path: PathBuf,
}

pub const MIN_PASSWORD_LEN: usize = 8;

/// Usernames: 1-32 of letters, digits, `_`, `-`, `.`.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

pub fn hash_password(password: &str) -> io::Result<String> {
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|h| h.to_string())
        .map_err(|e| io::Error::other(format!("could not hash password: {e}")))
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    match PasswordHash::new(hash) {
        Ok(parsed) => Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok(),
        Err(_) => false,
    }
}

impl Users {
    pub fn new(data: &Path) -> Self {
        Users { path: data.join("users.json") }
    }

    pub fn list(&self) -> io::Result<Vec<User>> {
        match std::fs::read_to_string(&self.path) {
            Ok(s) => serde_json::from_str(&s).map_err(io::Error::other),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }

    pub fn get(&self, name: &str) -> io::Result<Option<User>> {
        Ok(self.list()?.into_iter().find(|u| u.name.eq_ignore_ascii_case(name)))
    }

    fn save(&self, users: &[User]) -> io::Result<()> {
        let json = serde_json::to_string_pretty(users).map_err(io::Error::other)?;
        crate::write_private(&self.path, json.as_bytes())
    }

    pub fn add(&self, name: &str, password: &str, read_only: bool) -> io::Result<()> {
        if !valid_name(name) {
            return Err(invalid("username must be 1-32 letters, digits, _ - or ."));
        }
        check_password(password)?;
        let mut users = self.list()?;
        if users.iter().any(|u| u.name.eq_ignore_ascii_case(name)) {
            return Err(invalid("that user already exists"));
        }
        users.push(User { name: name.to_string(), hash: hash_password(password)?, read_only });
        self.save(&users)
    }

    pub fn set_password(&self, name: &str, password: &str) -> io::Result<()> {
        check_password(password)?;
        let mut users = self.list()?;
        let user = users
            .iter_mut()
            .find(|u| u.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| invalid("no such user"))?;
        user.hash = hash_password(password)?;
        self.save(&users)
    }

    pub fn set_read_only(&self, name: &str, read_only: bool) -> io::Result<()> {
        let mut users = self.list()?;
        let user = users
            .iter_mut()
            .find(|u| u.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| invalid("no such user"))?;
        user.read_only = read_only;
        self.save(&users)
    }

    pub fn remove(&self, name: &str) -> io::Result<()> {
        let mut users = self.list()?;
        let before = users.len();
        users.retain(|u| !u.name.eq_ignore_ascii_case(name));
        if users.len() == before {
            return Err(invalid("no such user"));
        }
        self.save(&users)
    }
}

fn check_password(password: &str) -> io::Result<()> {
    if password.chars().count() < MIN_PASSWORD_LEN {
        return Err(invalid("password must be at least 8 characters"));
    }
    Ok(())
}

fn invalid(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, msg.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_verify_remove() {
        let dir = tempfile::tempdir().unwrap();
        let users = Users::new(dir.path());
        users.add("alice", "correct horse", false).unwrap();
        assert!(users.add("ALICE", "another pass", false).is_err(), "names are case-insensitive");
        assert!(users.add("bob", "short", false).is_err());
        assert!(users.add("bad name", "long enough", false).is_err());

        let u = users.get("Alice").unwrap().unwrap();
        assert!(!u.hash.contains("correct horse"));
        assert!(verify_password("correct horse", &u.hash));
        assert!(!verify_password("wrong", &u.hash));

        users.set_password("alice", "new password").unwrap();
        let u = users.get("alice").unwrap().unwrap();
        assert!(verify_password("new password", &u.hash));
        users.remove("alice").unwrap();
        assert!(users.get("alice").unwrap().is_none());
    }
}
