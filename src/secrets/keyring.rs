use std::collections::HashMap;
use std::sync::Mutex;

use keyring::Entry;

use super::{SecretError, SecretRef, SecretString};

/// OS-independent credential store used by business logic.
pub trait SecretStore: Send + Sync {
    fn exists(&self, reference: &SecretRef) -> Result<bool, SecretError>;
    fn set(&self, reference: &SecretRef, secret: SecretString) -> Result<(), SecretError>;
    fn get(&self, reference: &SecretRef) -> Result<SecretString, SecretError>;
    fn delete(&self, reference: &SecretRef) -> Result<(), SecretError>;
}

/// In-memory store for unit/integration tests. Never touches the OS keyring.
#[derive(Default)]
pub struct FakeSecretStore {
    entries: Mutex<HashMap<String, String>>,
}

impl FakeSecretStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretStore for FakeSecretStore {
    fn exists(&self, reference: &SecretRef) -> Result<bool, SecretError> {
        let entries = self.entries.lock().map_err(|_| SecretError::Backend)?;
        Ok(entries.contains_key(reference.as_str()))
    }
    fn set(&self, reference: &SecretRef, secret: SecretString) -> Result<(), SecretError> {
        let mut entries = self.entries.lock().map_err(|_| SecretError::Backend)?;
        entries.insert(reference.as_str().to_owned(), secret.expose().to_owned());
        Ok(())
    }

    fn get(&self, reference: &SecretRef) -> Result<SecretString, SecretError> {
        let entries = self.entries.lock().map_err(|_| SecretError::Backend)?;
        entries
            .get(reference.as_str())
            .map(|value| SecretString::from(value.as_str()))
            .ok_or(SecretError::NotFound)
    }

    fn delete(&self, reference: &SecretRef) -> Result<(), SecretError> {
        let mut entries = self.entries.lock().map_err(|_| SecretError::Backend)?;
        if entries.remove(reference.as_str()).is_some() {
            Ok(())
        } else {
            Err(SecretError::NotFound)
        }
    }
}

/// Production adapter for the platform credential store.
///
/// Tests must not construct this type; use [`FakeSecretStore`] instead.
pub struct OsKeyringStore;

impl SecretStore for OsKeyringStore {
    fn exists(&self, reference: &SecretRef) -> Result<bool, SecretError> {
        match self.get(reference) {
            Ok(_) => Ok(true),
            Err(SecretError::NotFound) => Ok(false),
            Err(error) => Err(error),
        }
    }
    fn set(&self, reference: &SecretRef, secret: SecretString) -> Result<(), SecretError> {
        let entry = Entry::new(SecretRef::SERVICE_NAME, reference.as_str())
            .map_err(|_| SecretError::Backend)?;
        entry
            .set_password(secret.expose())
            .map_err(|_| SecretError::Backend)
    }

    fn get(&self, reference: &SecretRef) -> Result<SecretString, SecretError> {
        let entry = Entry::new(SecretRef::SERVICE_NAME, reference.as_str())
            .map_err(|_| SecretError::Backend)?;
        match entry.get_password() {
            Ok(password) => Ok(SecretString::from(password)),
            Err(keyring::Error::NoEntry) => Err(SecretError::NotFound),
            Err(_) => Err(SecretError::Backend),
        }
    }

    fn delete(&self, reference: &SecretRef) -> Result<(), SecretError> {
        let entry = Entry::new(SecretRef::SERVICE_NAME, reference.as_str())
            .map_err(|_| SecretError::Backend)?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Err(SecretError::NotFound),
            Err(_) => Err(SecretError::Backend),
        }
    }
}
