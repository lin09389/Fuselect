//! System credential boundary. Secrets never enter SQLite or public logs.

mod keyring;

pub use keyring::{FakeSecretStore, OsKeyringStore, SecretStore};

use std::fmt::{Debug, Display, Formatter};

use secrecy::{ExposeSecret, SecretBox};
use zeroize::Zeroize;

/// Stable, non-secret Keyring entry name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SecretRef {
    name: String,
}

impl SecretRef {
    pub const SERVICE_NAME: &'static str = "fuselect";

    pub fn new(name: impl Into<String>) -> Result<Self, SecretError> {
        let name = name.into();
        if name.trim().is_empty() || name.contains('\0') {
            return Err(SecretError::InvalidReference);
        }
        Ok(Self { name })
    }

    pub fn worker(worker_id: &str) -> Result<Self, SecretError> {
        Self::new(format!("fuselect/worker/{worker_id}"))
    }

    pub fn gateway_default() -> Self {
        Self {
            name: "fuselect/gateway/default".to_owned(),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.name
    }
}

impl Display for SecretRef {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.name)
    }
}

/// Secret payload that redacts Debug and zeroizes on drop.
pub struct SecretString {
    inner: SecretBox<str>,
}

impl SecretString {
    pub fn new(mut value: String) -> Self {
        let boxed = SecretBox::from(value.clone());
        value.zeroize();
        Self { inner: boxed }
    }

    pub fn expose(&self) -> &str {
        self.inner.expose_secret()
    }
}

impl Debug for SecretString {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SecretString([REDACTED])")
    }
}

impl From<&str> for SecretString {
    fn from(value: &str) -> Self {
        Self::new(value.to_owned())
    }
}

impl From<String> for SecretString {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SecretError {
    #[error("密钥引用无效")]
    InvalidReference,
    #[error("密钥不存在")]
    NotFound,
    #[error("系统密钥库操作失败")]
    Backend,
    #[error("密钥删除失败，Worker 已禁用，请重试清理")]
    CleanupPending,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_debug_is_redacted() {
        let secret = SecretString::from("TOP_SECRET_API_KEY");
        let rendered = format!("{secret:?}");
        assert_eq!(rendered, "SecretString([REDACTED])");
        assert!(!rendered.contains("TOP_SECRET_API_KEY"));
    }

    #[test]
    fn worker_secret_ref_is_stable() {
        let reference = SecretRef::worker("coder-a").unwrap();
        assert_eq!(reference.as_str(), "fuselect/worker/coder-a");
    }
}
