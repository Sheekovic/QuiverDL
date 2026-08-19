use keyring::{Entry, Error as KeyringError};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

const SERVICE: &str = "app.quiverdl.proxy";
const ACCOUNT: &str = "default";
const MAX_USERNAME_CHARS: usize = 512;
const MAX_PASSWORD_BYTES: usize = 8 * 1024;

#[derive(Deserialize)]
struct StoredCredential {
    username: String,
    password: String,
}

impl Drop for StoredCredential {
    fn drop(&mut self) {
        self.password.zeroize();
    }
}

#[derive(Serialize)]
struct CredentialRef<'a> {
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

#[tauri::command]
pub(crate) async fn save_proxy_credentials(
    username: String,
    password: String,
) -> Result<(), String> {
    validate(&username, &password)?;
    let username = Zeroizing::new(username);
    let password = Zeroizing::new(password);
    tokio::task::spawn_blocking(move || {
        let payload = CredentialRef {
            username: username.as_str(),
            password: password.as_str(),
        };
        let encoded = Zeroizing::new(
            serde_json::to_vec(&payload)
                .map_err(|_| "Could not prepare proxy credentials for secure storage")?,
        );
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
pub(crate) async fn has_proxy_credentials(username: String) -> Result<bool, String> {
    if username.is_empty() {
        return Ok(false);
    }
    tokio::task::spawn_blocking(move || {
        Ok(load_stored()?.is_some_and(|stored| stored.username == username))
    })
    .await
    .map_err(|_| "The proxy credential task could not be completed".to_string())?
}

pub(crate) async fn load_proxy_password(username: String) -> Result<Option<String>, String> {
    if username.is_empty() {
        return Ok(None);
    }
    tokio::task::spawn_blocking(move || {
        let Some(mut stored) = load_stored()? else {
            return Ok(None);
        };
        if stored.username != username {
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
    use super::validate;

    #[test]
    fn rejects_ambiguous_or_control_character_credentials() {
        assert!(validate("", "secret").is_err());
        assert!(validate("user:name", "secret").is_err());
        assert!(validate("user", "line\nbreak").is_err());
        assert!(validate("user", "secret").is_ok());
    }
}
