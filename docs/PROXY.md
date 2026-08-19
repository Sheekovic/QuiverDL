# Proxy routing

QuiverDL supports three explicit routing modes in Desktop Settings:

- **Direct connection** is the default and ignores ambient proxy environment settings.
- **System proxy** lets the HTTP client discover the operating system or environment proxy.
- **Custom proxy** sends HTTP and HTTPS downloads through one HTTP(S) proxy endpoint.

The same route is used for inspection requests, redirects, resume validation, and every parallel
segment so a download cannot accidentally switch between direct and proxied connections.

## Custom proxy configuration

Enter an endpoint such as `http://proxy.example:8080`. User information, paths, queries, and
fragments are rejected in this field. SOCKS and PAC scripts are not supported in this cycle.
Choose **Apply proxy settings** after editing; QuiverDL validates the draft before it replaces the
active, persisted route.

The optional comma-separated bypass list uses conventional `NO_PROXY` matching, for example:

```text
localhost, 127.0.0.1, .internal.example
```

Bypassed hosts connect directly. Keep the list narrow: bypassing a host also bypasses proxy policy,
filtering, and monitoring for that destination.

## Authentication

Leave the username empty for an unauthenticated proxy. For HTTP Basic proxy authentication, enter a
username and password and choose **Save password securely**. QuiverDL stores one proxy credential in
the operating-system credential service:

- Windows Credential Manager
- macOS Keychain
- Linux Secret Service

Only the username and endpoint are written to QuiverDL's `state.json`. The password is never added
to the proxy URL or application state and is not returned to the webview after it has been saved.
Changing the username requires saving its password again. **Remove** deletes the QuiverDL proxy
entry from the credential service. Credentials are bound to both the normalized endpoint and
username, so changing either value fails closed until a password is explicitly saved for the new
combination.

If the desktop credential service is locked or unavailable, authenticated custom routing fails
closed with a generic error. QuiverDL does not fall back to plaintext storage or silently connect
directly.

## Privacy boundary

An HTTP proxy can read and change plain HTTP downloads. For HTTPS downloads, the proxy sees the
destination and connection timing while TLS protects content unless the device trusts an
interception certificate controlled by the proxy. A proxy is therefore part of the trusted network
path, not an anonymity guarantee.
