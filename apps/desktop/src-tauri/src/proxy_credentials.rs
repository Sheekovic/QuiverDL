use keyring::{Entry, Error as KeyringError};
use serde::{Deserialize, Serialize};
use url::Url;
use zeroize::{Zeroize, Zeroizing};

const SERVICE: &str = "app.quiverdl.proxy";
const ACCOUNT: &str = "default";
const MAX_USERNAME_CHARS: usize = 512;
const MAX_PASSWORD_BYTES: usize = 8 * 1024;
const MAX_STORED_CREDENTIAL_BYTES: usize = 2_560;

#[derive(Deserialize)]
struct StoredCredential {
    endpoint: String,
    username: String,
    password: String,
}

impl StoredCredential {
    fn matches(&self, endpoint: &str, username: &str) -> bool {
        self.endpoint == endpoint && self.username == username
    }
}

impl Drop for StoredCredential {
    fn drop(&mut self) {
        self.password.zeroize();
    }
}

#[derive(Serialize)]
struct CredentialRef<'a> {
    endpoint: &'a str,
    username: &'a str,
    password: &'a str,
}

fn validate(username: &str, password: &str) -> Result<(), String> {
    if username.is_empty()
        || username.chars().count() > MAX_USERNAME_CHARS
        || username.contains(':')
        || username.chars().any(char::is_control)
    {
        return Err(
            "The proxy username is empty, too long, or contains unsupported characters".into(),
        );
    }
    if password.is_empty()
        || password.len() > MAX_PASSWORD_BYTES
        || password.chars().any(char::is_control)
    {
        return Err(
            "The proxy password is empty, too long, or contains unsupported characters".into(),
        );
    }
    Ok(())
}

fn entry() -> Result<Entry, String> {
    Entry::new(SERVICE, ACCOUNT)
        .map_err(|_| "The operating-system credential store is unavailable".into())
}

fn normalize_endpoint(endpoint: &str) -> Result<String, String> {
    let endpoint = Url::parse(endpoint.trim()).map_err(|_| "The custom proxy URL is invalid")?;
    quiver_core::ProxyConfig::new(endpoint.clone()).map_err(|error| error.to_string())?;
    Ok(endpoint.to_string())
}

fn encode_credential(
    endpoint: &str,
    username: &str,
    password: &str,
) -> Result<Zeroizing<Vec<u8>>, String> {
    let payload = CredentialRef {
        endpoint,
        username,
        password,
    };
    let encoded = Zeroizing::new(
        serde_json::to_vec(&payload)
            .map_err(|_| "Could not prepare proxy credentials for secure storage")?,
    );
    if encoded.len() > MAX_STORED_CREDENTIAL_BYTES {
        return Err(
            "The proxy username and password are too long for the operating-system credential store"
                .into(),
        );
    }
    Ok(encoded)
}

#[tauri::command]
pub(crate) async fn save_proxy_credentials(
    endpoint: String,
    username: String,
    password: String,
) -> Result<(), String> {
    validate(&username, &password)?;
    let endpoint = Zeroizing::new(normalize_endpoint(&endpoint)?);
    let username = Zeroizing::new(username);
    let password = Zeroizing::new(password);
    tokio::task::spawn_blocking(move || {
        let encoded = encode_credential(endpoint.as_str(), username.as_str(), password.as_str())?;
        entry()?.set_secret(encoded.as_slice()).map_err(|_| {
            "Could not save proxy credentials in the operating-system credential store".into()
        })
    })
    .await
    .map_err(|_| "The proxy credential task could not be completed".to_string())?
}

#[tauri::command]
pub(crate) async fn clear_proxy_credentials() -> Result<(), String> {
    tokio::task::spawn_blocking(move || match entry()?.delete_credential() {
        Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
        Err(_) => Err(
            "Could not remove proxy credentials from the operating-system credential store".into(),
        ),
    })
    .await
    .map_err(|_| "The proxy credential task could not be completed".to_string())?
}

#[tauri::command]
pub(crate) async fn has_proxy_credentials(
    endpoint: String,
    username: String,
) -> Result<bool, String> {
    if endpoint.is_empty() || username.is_empty() {
        return Ok(false);
    }
    let Ok(endpoint) = normalize_endpoint(&endpoint) else {
        return Ok(false);
    };
    tokio::task::spawn_blocking(move || {
        Ok(load_stored()?.is_some_and(|stored| stored.matches(&endpoint, &username)))
    })
    .await
    .map_err(|_| "The proxy credential task could not be completed".to_string())?
}

pub(crate) async fn load_proxy_password(
    endpoint: String,
    username: String,
) -> Result<Option<String>, String> {
    if endpoint.is_empty() || username.is_empty() {
        return Ok(None);
    }
    tokio::task::spawn_blocking(move || {
        let Some(mut stored) = load_stored()? else {
            return Ok(None);
        };
        if !stored.matches(&endpoint, &username) {
            stored.password.zeroize();
            return Ok(None);
        }
        Ok(Some(std::mem::take(&mut stored.password)))
    })
    .await
    .map_err(|_| "The proxy credential task could not be completed".to_string())?
}

fn load_stored() -> Result<Option<StoredCredential>, String> {
    let secret = match entry()?.get_secret() {
        Ok(secret) => Zeroizing::new(secret),
        Err(KeyringError::NoEntry) => return Ok(None),
        Err(_) => {
            return Err(
                "Could not read proxy credentials from the operating-system credential store"
                    .into(),
            );
        }
    };
    let stored = serde_json::from_slice(secret.as_slice())
        .map_err(|_| "The stored proxy credential is invalid")?;
    Ok(Some(stored))
}

#[cfg(test)]
mod tests {
    use super::{StoredCredential, encode_credential, normalize_endpoint, validate};

    #[test]
    fn rejects_ambiguous_or_control_character_credentials() {
        assert!(validate("", "secret").is_err());
        assert!(validate("user:name", "secret").is_err());
        assert!(validate("user", "line\nbreak").is_err());
        assert!(validate("user", "secret").is_ok());
    }

    #[test]
    fn rejects_a_serialized_credential_above_the_windows_limit() {
        let password = "\"".repeat(1_300);
        assert!(validate("user", &password).is_ok());
        let error = encode_credential("http://proxy.example:8080/", "user", &password)
            .expect_err("JSON escaping must not exceed the cross-platform keyring limit");
        assert!(error.contains("too long"));
        assert!(!error.contains(&password));
    }

    #[test]
    fn normalizes_and_rejects_unsafe_proxy_endpoints() {
        assert_eq!(
            normalize_endpoint("HTTP://Proxy.Example:8080").unwrap(),
            "http://proxy.example:8080/"
        );
        assert!(normalize_endpoint("http://user:secret@proxy.example").is_err());
    }

    #[test]
    fn stored_credentials_match_both_endpoint_and_username() {
        let stored = StoredCredential {
            endpoint: "http://proxy-a.example:8080/".into(),
            username: "user".into(),
            password: "secret".into(),
        };
        assert!(stored.matches("http://proxy-a.example:8080/", "user"));
        assert!(!stored.matches("http://proxy-b.example:8080/", "user"));
        assert!(!stored.matches("http://proxy-a.example:8080/", "another-user"));
    }
}
