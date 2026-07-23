use fuselect::secrets::{FakeSecretStore, SecretError, SecretRef, SecretStore, SecretString};

#[test]
fn fake_secret_store_set_get_delete() {
    let store = FakeSecretStore::new();
    let reference = SecretRef::worker("coder-a").unwrap();
    store
        .set(&reference, SecretString::from("TOP_SECRET_API_KEY"))
        .unwrap();

    let loaded = store.get(&reference).unwrap();
    assert_eq!(loaded.expose(), "TOP_SECRET_API_KEY");
    assert!(!format!("{loaded:?}").contains("TOP_SECRET_API_KEY"));
    assert!(!format!("{reference:?}").contains("TOP_SECRET_API_KEY"));

    store.delete(&reference).unwrap();
    assert_eq!(store.get(&reference).unwrap_err(), SecretError::NotFound);
}

#[test]
fn secret_errors_do_not_embed_secret_values() {
    let err = SecretError::Backend;
    let rendered = format!("{err:?} :: {err}");
    assert!(!rendered.contains("TOP_SECRET_API_KEY"));
    assert!(!rendered.contains("TOP_SECRET_GATEWAY_KEY"));
}

#[test]
fn gateway_secret_ref_is_stable_and_non_secret() {
    let reference = SecretRef::gateway_default();
    assert_eq!(reference.as_str(), "fuselect/gateway/default");
    assert_eq!(SecretRef::SERVICE_NAME, "fuselect");
}
