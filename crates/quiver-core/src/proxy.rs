use std::fmt;

use url::Url;
use zeroize::Zeroizing;

use crate::{Error, Result};

const MAX_PROXY_USERNAME_CHARS: usize = 512;
const MAX_PROXY_PASSWORD_BYTES: usize = 8 * 1024;
const MAX_BYPASS_LIST_CHARS: usize = 8 * 1024;

/// Determines how the engine routes HTTP and HTTPS requests.
#[derive(Clone, Debug, Default)]
pub enum ProxyPolicy {
    /// Never consult ambient operating-system or environment proxy settings.
    #[default]
    Disabled,
    /// Use the proxy settings discovered by the HTTP client from the environment.
    System,
    /// Route through one explicitly configured HTTP or HTTPS proxy.
    Custom(ProxyConfig),
}

/// A custom proxy endpoint with optional basic authentication.
///
/// Credentials are deliberately kept separate from the URL so formatting or
/// serializing the endpoint cannot expose them.
#[derive(Clone)]
pub struct ProxyConfig {
    endpoint: Url,
    bypass: Option<String>,
    credentials: Option<ProxyCredentials>,
}

#[derive(Clone)]
struct ProxyCredentials {
    username: String,
    password: Zeroizing<String>,
}

impl fmt::Debug for ProxyConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProxyConfig")
            .field("endpoint", &self.endpoint)
            .field("bypass", &self.bypass)
            .field("authenticated", &self.credentials.is_some())
            .finish()
    }
}

impl ProxyConfig {
    pub fn new(endpoint: Url) -> Result<Self> {
        validate_endpoint(&endpoint)?;
        Ok(Self {
            endpoint,
            bypass: None,
            credentials: None,
        })
    }

    pub fn with_basic_auth(
        mut self,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Result<Self> {
        let username = username.into();
        let password = password.into();
        if username.is_empty()
            || username.chars().count() > MAX_PROXY_USERNAME_CHARS
            || username.contains(':')
            || username.chars().any(char::is_control)
        {
            return Err(Error::InvalidProxyConfiguration(
                "the proxy username is empty, too long, or contains unsupported characters".into(),
            ));
        }
        if password.is_empty()
            || password.len() > MAX_PROXY_PASSWORD_BYTES
            || password.chars().any(char::is_control)
        {
            return Err(Error::InvalidProxyConfiguration(
                "the proxy password is empty, too long, or contains unsupported characters".into(),
            ));
        }
        self.credentials = Some(ProxyCredentials {
            username,
            password: Zeroizing::new(password),
        });
        Ok(self)
    }

    pub fn with_bypass_list(mut self, bypass: impl Into<String>) -> Result<Self> {
        let bypass = bypass.into();
        let bypass = bypass.trim();
        if bypass.is_empty() {
            self.bypass = None;
            return Ok(self);
        }
        if bypass.chars().count() > MAX_BYPASS_LIST_CHARS || bypass.chars().any(char::is_control) {
            return Err(Error::InvalidProxyConfiguration(
                "the proxy bypass list is too long or contains unsupported characters".into(),
            ));
        }
        if reqwest::NoProxy::from_string(bypass).is_none() {
            return Err(Error::InvalidProxyConfiguration(
                "the proxy bypass list is invalid".into(),
            ));
        }
        self.bypass = Some(bypass.to_owned());
        Ok(self)
    }

    pub(crate) fn to_reqwest(&self) -> Result<reqwest::Proxy> {
        let mut proxy = reqwest::Proxy::all(self.endpoint.as_str()).map_err(|_| {
            Error::InvalidProxyConfiguration("the proxy endpoint could not be configured".into())
        })?;
        if let Some(bypass) = &self.bypass {
            proxy = proxy.no_proxy(reqwest::NoProxy::from_string(bypass));
        }
        if let Some(credentials) = &self.credentials {
            proxy = proxy.basic_auth(&credentials.username, credentials.password.as_str());
        }
        Ok(proxy)
    }
}

fn validate_endpoint(endpoint: &Url) -> Result<()> {
    if !matches!(endpoint.scheme(), "http" | "https") {
        return Err(Error::InvalidProxyConfiguration(
            "only HTTP and HTTPS proxy endpoints are supported".into(),
        ));
    }
    if endpoint.host_str().is_none() {
        return Err(Error::InvalidProxyConfiguration(
            "the proxy endpoint must include a host".into(),
        ));
    }
    if !endpoint.username().is_empty() || endpoint.password().is_some() {
        return Err(Error::InvalidProxyConfiguration(
            "credentials must not be embedded in the proxy URL".into(),
        ));
    }
    if endpoint.query().is_some() || endpoint.fragment().is_some() {
        return Err(Error::InvalidProxyConfiguration(
            "the proxy endpoint must not contain a query or fragment".into(),
        ));
    }
    if !matches!(endpoint.path(), "" | "/") {
        return Err(Error::InvalidProxyConfiguration(
            "the proxy endpoint must not contain a path".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use url::Url;

    use super::ProxyConfig;

    #[test]
    fn debug_output_redacts_basic_auth() {
        let config = ProxyConfig::new(Url::parse("http://proxy.example:8080").unwrap())
            .unwrap()
            .with_basic_auth("alice", "correct horse battery staple")
            .unwrap();
        let output = format!("{config:?}");
        assert!(output.contains("authenticated: true"));
        assert!(!output.contains("alice"));
        assert!(!output.contains("correct horse"));
    }

    #[test]
    fn rejects_credentials_embedded_in_the_endpoint() {
        let error = ProxyConfig::new(Url::parse("http://alice:secret@proxy.example:8080").unwrap())
            .unwrap_err();
        assert!(error.to_string().contains("must not be embedded"));
        assert!(!error.to_string().contains("secret"));
    }

    #[test]
    fn accepts_a_bounded_bypass_list() {
        ProxyConfig::new(Url::parse("http://proxy.example:8080").unwrap())
            .unwrap()
            .with_bypass_list("localhost, 127.0.0.1, .internal.example")
            .unwrap();
    }
}
