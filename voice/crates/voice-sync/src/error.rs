//! What can go wrong while syncing, and the codes Settings shows for it.

use std::fmt;

use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncErrorKind {
    Endpoint,
    Network,
    Timeout,
    Auth,
    Bucket,
    Permission,
    Clock,
    Throttled,
    Server,
    NoSuchUpload,
    NotFound,
    NoSecret,
    Off,
    Key,
    /// The system wouldn't let Voice open its key now: the user chose Deny in macOS's keychain
    /// dialog, or nobody could answer it. The key is still there; allowing Voice opens it again.
    KeyDenied,
    KeyInUse,
    NotInBucket,
    Crypto,
    Local,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncError {
    pub kind: SyncErrorKind,
    /// English, for logs and Error details. Never holds the secret or a signed URL.
    pub message: String,
}

impl SyncError {
    pub fn new(kind: SyncErrorKind, message: impl Into<String>) -> SyncError {
        SyncError {
            kind,
            message: message.into(),
        }
    }

    /// The code Settings maps to text: endpoint, network, auth, bucket, clock, permission,
    /// no_secret, key, key_denied, key_in_use, off or failed. A timeout counts as `network`: in both cases
    /// Voice couldn't get an answer from the endpoint.
    pub fn code(&self) -> &'static str {
        match self.kind {
            SyncErrorKind::Endpoint => "endpoint",
            SyncErrorKind::Network | SyncErrorKind::Timeout => "network",
            SyncErrorKind::Auth => "auth",
            SyncErrorKind::Bucket => "bucket",
            SyncErrorKind::Clock => "clock",
            SyncErrorKind::Permission => "permission",
            SyncErrorKind::NoSecret => "no_secret",
            SyncErrorKind::Key => "key",
            SyncErrorKind::KeyDenied => "key_denied",
            SyncErrorKind::KeyInUse => "key_in_use",
            SyncErrorKind::Off => "off",
            SyncErrorKind::Throttled
            | SyncErrorKind::Server
            | SyncErrorKind::NoSuchUpload
            | SyncErrorKind::NotFound
            | SyncErrorKind::NotInBucket
            | SyncErrorKind::Crypto
            | SyncErrorKind::Local => "failed",
        }
    }

    /// Whether waiting and trying again can help: network, timeout, throttled, server.
    pub fn transient(&self) -> bool {
        matches!(
            self.kind,
            SyncErrorKind::Network
                | SyncErrorKind::Timeout
                | SyncErrorKind::Throttled
                | SyncErrorKind::Server
        )
    }
}

impl SyncErrorKind {
    /// Voice can't open its key or the bucket secret on this computer: the key is unreadable or
    /// missing (`Key`), or the system wouldn't let Voice open it now (`KeyDenied`). Both stop
    /// uploads and show as `sync.state` `key`.
    pub fn is_key(self) -> bool {
        matches!(self, SyncErrorKind::Key | SyncErrorKind::KeyDenied)
    }
}

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for SyncError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_a_settings_code() {
        use SyncErrorKind::*;
        let cases = [
            (Endpoint, "endpoint", false),
            (Network, "network", true),
            (Timeout, "network", true),
            (Auth, "auth", false),
            (Bucket, "bucket", false),
            (Permission, "permission", false),
            (Clock, "clock", false),
            (Throttled, "failed", true),
            (Server, "failed", true),
            (NoSuchUpload, "failed", false),
            (NotFound, "failed", false),
            (NoSecret, "no_secret", false),
            (Off, "off", false),
            (Key, "key", false),
            (KeyDenied, "key_denied", false),
            (KeyInUse, "key_in_use", false),
            (NotInBucket, "failed", false),
            (Crypto, "failed", false),
            (Local, "failed", false),
        ];
        for (kind, code, transient) in cases {
            let error = SyncError::new(kind, "x");
            assert_eq!(error.code(), code, "{kind:?}");
            assert_eq!(error.transient(), transient, "{kind:?}");
        }
        assert_eq!(
            SyncError::new(Off, "Sync is off.").to_string(),
            "Sync is off."
        );
        assert_eq!(
            serde_json::to_string(&NoSuchUpload).unwrap(),
            "\"no_such_upload\""
        );
    }
}
