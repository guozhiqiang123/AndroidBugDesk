use crate::{model::Result, store::Store};
use serde_json::json;
use zeroize::Zeroize;

// A separate opt-in prevents legacy credentials from being restored before
// the user chooses the new remember-login checkbox.
pub fn enabled(store: &Store) -> Result<bool> {
    Ok(store
        .get::<bool>("settings", "remember_login")?
        .unwrap_or(false))
}

pub fn set_enabled(store: &Store, remember: bool) -> Result<()> {
    let preference = json!(remember);
    store.put_many(&[
        ("settings", "remembered", &preference),
        ("settings", "remember_login", &preference),
    ])
}

fn load_with(store: &Store, read: impl FnOnce() -> Result<String>) -> Result<Option<String>> {
    if !enabled(store)? {
        return Ok(None);
    }
    let Ok(mut token) = read() else {
        return Ok(None);
    };
    if token.trim().len() < 8 || token.len() > 4096 || token.contains(['\n', '\r']) {
        token.zeroize();
        return Ok(None);
    }
    Ok(Some(token))
}

pub fn load_token(store: &Store) -> Result<Option<String>> {
    if store.get::<String>("settings", "login_method")?.as_deref() == Some("oauth") {
        return Ok(None);
    }
    load_with(store, || {
        keyring::Entry::new(crate::commands::SERVICE, "personal-pat")
            .and_then(|entry| entry.get_password())
            .map_err(|_| "无法读取系统保存的登录令牌".into())
    })
}

pub fn clear_pat() -> Result<()> {
    match keyring::Entry::new(crate::commands::SERVICE, "personal-pat")
        .and_then(|entry| entry.delete_credential())
    {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err("无法移除系统保存的令牌，请检查安全存储".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unchecked_and_legacy_preferences_never_read_credentials() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path()).unwrap();
        store.put("settings", "remembered", &true).unwrap();
        assert!(
            load_with(&store, || panic!("legacy flag must not read credentials"))
                .unwrap()
                .is_none()
        );
        set_enabled(&store, false).unwrap();
        assert!(
            load_with(&store, || panic!("unchecked must not read credentials"))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn opted_in_restart_restores_only_from_safe_storage() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path()).unwrap();
        set_enabled(&store, true).unwrap();
        drop(store);
        let reopened = Store::new(root.path()).unwrap();
        assert_eq!(
            load_with(&reopened, || Ok("mock-login-token".into())).unwrap(),
            Some("mock-login-token".into())
        );
        assert!(!serde_json::to_string(&reopened.list("settings").unwrap())
            .unwrap()
            .contains("mock-login-token"));
        set_enabled(&reopened, false).unwrap();
        assert!(!enabled(&reopened).unwrap());
    }

    #[test]
    fn unavailable_or_invalid_credentials_fall_back_to_manual_login() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path()).unwrap();
        set_enabled(&store, true).unwrap();
        assert!(load_with(&store, || Err("unavailable".into()))
            .unwrap()
            .is_none());
        assert!(load_with(&store, || Ok("bad\ntoken".into()))
            .unwrap()
            .is_none());
    }
}
