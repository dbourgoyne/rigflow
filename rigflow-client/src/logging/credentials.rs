//! Per-service credential storage for online sync (currently LoTW).
//!
//! Tries the OS keyring first (macOS Keychain / Linux Secret Service). When that
//! is unavailable — common on a minimal Linux box with no Secret Service daemon —
//! it falls back to a per-operator file with owner-only (`0600`) permissions.
//! The backend actually used is reported so the UI can say where the secret went.
//!
//! Login and password are stored together as one secret (`login\npassword`); the
//! login isn't secret, but pairing them keeps one source of truth and one place
//! to clear. Nothing here logs or returns the password.

use std::path::Path;

/// A LoTW login pair.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Credential {
    pub login: String,
    pub password: String,
}

/// Which store a credential lives in, for a one-line UI note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Keyring,
    File,
}

impl Backend {
    pub fn note(self) -> &'static str {
        match self {
            Backend::Keyring => "saved in the OS keyring",
            Backend::File => "OS keyring unavailable — saved in a local file (owner-only)",
        }
    }
}

/// The keyring "service" namespace (distinct from a ham *service* like LoTW).
const KEYRING_APP: &str = "rigflow";

/// Keyring account key for `(service, operator)`, e.g. `lotw:KK7TCY`.
fn account(service: &str, operator: &str) -> String {
    format!("{service}:{operator}")
}

fn file_path(dir: &Path, service: &str) -> std::path::PathBuf {
    dir.join(format!("{service}.cred"))
}

fn pack(c: &Credential) -> String {
    format!("{}\n{}", c.login, c.password)
}

fn unpack(s: &str) -> Credential {
    let s = s.strip_suffix('\n').unwrap_or(s);
    let (login, password) = s.split_once('\n').unwrap_or((s, ""));
    Credential {
        login: login.to_string(),
        password: password.to_string(),
    }
}

/// Store `cred` for `service` under `operator`. Returns which backend took it.
/// Prefers the keyring; on any keyring failure, writes the owner-only file.
pub fn store(
    service: &str,
    operator: &str,
    cred: &Credential,
    file_dir: &Path,
) -> Result<Backend, String> {
    if let Ok(entry) = keyring::Entry::new(KEYRING_APP, &account(service, operator))
        && entry.set_password(&pack(cred)).is_ok()
    {
        // Don't leave a stale plaintext copy once the keyring holds it.
        let _ = std::fs::remove_file(file_path(file_dir, service));
        return Ok(Backend::Keyring);
    }
    let path = file_path(file_dir, service);
    std::fs::write(&path, pack(cred).as_bytes()).map_err(|e| format!("saving credential: {e}"))?;
    set_owner_only(&path);
    Ok(Backend::File)
}

/// Load a stored credential, if any, and where it came from.
pub fn load(service: &str, operator: &str, file_dir: &Path) -> Option<(Credential, Backend)> {
    if let Ok(entry) = keyring::Entry::new(KEYRING_APP, &account(service, operator))
        && let Ok(secret) = entry.get_password()
    {
        return Some((unpack(&secret), Backend::Keyring));
    }
    std::fs::read_to_string(file_path(file_dir, service))
        .ok()
        .map(|s| (unpack(&s), Backend::File))
}

/// Forget a stored credential from whichever backend holds it.
pub fn clear(service: &str, operator: &str, file_dir: &Path) {
    if let Ok(entry) = keyring::Entry::new(KEYRING_APP, &account(service, operator)) {
        let _ = entry.delete_credential();
    }
    let _ = std::fs::remove_file(file_path(file_dir, service));
}

#[cfg(unix)]
fn set_owner_only(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}
#[cfg(not(unix))]
fn set_owner_only(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_unpack_round_trips() {
        let c = Credential {
            login: "KK7TCY".into(),
            password: "s3cr3t pass".into(),
        };
        assert_eq!(unpack(&pack(&c)), c);
    }

    #[test]
    fn file_fallback_round_trips_and_is_owner_only() {
        let dir = std::env::temp_dir().join(format!("rigflow-cred-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Force the file path directly (the keyring may or may not exist in CI).
        let c = Credential {
            login: "KK7TCY".into(),
            password: "pw".into(),
        };
        let path = file_path(&dir, "lotw");
        std::fs::write(&path, pack(&c).as_bytes()).unwrap();
        set_owner_only(&path);
        let got = unpack(&std::fs::read_to_string(&path).unwrap());
        assert_eq!(got, c);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
