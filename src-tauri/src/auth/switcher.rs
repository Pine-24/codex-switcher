//! Account switching logic - writes credentials to ~/.codex/auth.json

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::Utc;

use crate::types::{
    parse_chatgpt_id_token_claims, AuthData, AuthDotJson, StoredAccount, TokenData,
};

/// Get the official Codex home directory
pub fn get_codex_home() -> Result<PathBuf> {
    let selection = super::home_detection::resolve()?;
    let path = selection.path.context(
        selection
            .error
            .unwrap_or_else(|| "Select a Codex account folder in Settings".into()),
    )?;
    validate_codex_home(&path)
}

pub fn validate_codex_home(value: &str) -> Result<PathBuf> {
    #[cfg(windows)]
    let normalized = value.replace('/', "\\");
    #[cfg(windows)]
    let value = normalized.as_str();
    let path = PathBuf::from(value);
    if value.trim().is_empty() || !path.is_absolute() || value.contains('\0') {
        anyhow::bail!("Codex home must be an absolute directory path");
    }
    #[cfg(windows)]
    if value.starts_with(r"\\") && wsl_distribution(value).is_none() {
        anyhow::bail!("Only local drives and WSL directories are supported for Codex home");
    }
    Ok(path)
}

#[cfg(any(windows, test))]
pub(crate) fn wsl_distribution(path: &str) -> Option<&str> {
    let mut parts = path.strip_prefix(r"\\")?.split('\\');
    let host = parts.next()?;
    if !host.eq_ignore_ascii_case("wsl.localhost") && !host.eq_ignore_ascii_case("wsl$") {
        return None;
    }
    let distribution = parts.next()?;
    if distribution.is_empty()
        || distribution.starts_with('-')
        || matches!(distribution, "." | "..")
        || !distribution.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
        || parts.next().map_or(true, str::is_empty)
    {
        return None;
    }
    Some(distribution)
}

pub(crate) fn ensure_auth_target_stopped() -> Result<()> {
    anyhow::ensure!(
        super::wsl::blocking_count()? == 0,
        "Cannot switch accounts while Codex processes are running in the selected WSL distribution"
    );
    Ok(())
}

pub fn set_codex_home_override(path: Option<String>) -> Result<()> {
    let path = path.filter(|value| !value.trim().is_empty());
    let path = path.map(|value| validate_codex_home(&value)).transpose()?;
    if path.as_ref().is_some_and(|directory| !directory.is_dir()) {
        anyhow::bail!("Codex home must be an existing, accessible directory");
    }
    let mut settings = super::load_app_settings()?;
    let path = path.map(|directory| directory.to_string_lossy().into_owned());
    if settings.codex_home_override == path {
        return Ok(());
    }
    let mut accounts = super::load_accounts()?;
    if get_codex_home().is_ok() {
        if let Some(auth) = read_current_auth()? {
            super::sync_active_account_tokens(&mut accounts, &auth);
        }
    }
    accounts.active_account_id = None;
    super::save_accounts(&accounts)?;
    settings.codex_home_override = path;
    super::save_app_settings(&settings)?;
    super::home_detection::clear_cache()
}

pub fn rediscover_codex_home() -> Result<()> {
    let mut accounts = super::load_accounts()?;
    if get_codex_home().is_ok() {
        if let Some(auth) = read_current_auth()? {
            super::sync_active_account_tokens(&mut accounts, &auth);
        }
    }
    accounts.active_account_id = None;
    super::save_accounts(&accounts)?;
    let mut settings = super::load_app_settings()?;
    settings.codex_home_override = None;
    super::save_app_settings(&settings)?;
    super::home_detection::clear_cache()
}

pub fn get_codex_auth_file() -> Result<PathBuf> {
    Ok(get_codex_home()?.join("auth.json"))
}

/// Switch to a specific account by writing its credentials to ~/.codex/auth.json
pub fn switch_to_account(account: &StoredAccount) -> Result<()> {
    ensure_auth_target_stopped()?;
    let codex_home = get_codex_home()?;

    // Ensure the codex home directory exists
    fs::create_dir_all(&codex_home)
        .with_context(|| format!("Failed to create codex home: {}", codex_home.display()))?;

    let auth_json = create_auth_json(account)?;

    let auth_path = codex_home.join("auth.json");
    let content =
        serde_json::to_string_pretty(&auth_json).context("Failed to serialize auth.json")?;

    #[cfg(windows)]
    let create = wsl_distribution(&codex_home.to_string_lossy()).is_none();
    #[cfg(not(windows))]
    let create = true;
    let mut options = fs::OpenOptions::new();
    options.write(true).truncate(true).create(create);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&auth_path).with_context(|| {
        format!(
            "Failed to open auth.json (WSL requires an existing login): {}",
            auth_path.display()
        )
    })?;
    file.write_all(content.as_bytes())
        .with_context(|| format!("Failed to write auth.json: {}", auth_path.display()))?;

    // Set restrictive permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o600);
        fs::set_permissions(&auth_path, perms)?;
    }

    Ok(())
}

/// Create an AuthDotJson structure from a StoredAccount
fn create_auth_json(account: &StoredAccount) -> Result<AuthDotJson> {
    match &account.auth_data {
        AuthData::ApiKey { key } => Ok(AuthDotJson {
            openai_api_key: Some(key.clone()),
            tokens: None,
            last_refresh: None,
        }),
        AuthData::ChatGPT {
            id_token,
            access_token,
            refresh_token,
            account_id,
        } => Ok(AuthDotJson {
            openai_api_key: None,
            tokens: Some(TokenData {
                id_token: id_token.clone(),
                access_token: access_token.clone(),
                refresh_token: refresh_token.clone(),
                account_id: account_id.clone(),
            }),
            last_refresh: Some(Utc::now()),
        }),
    }
}

/// Import an account from an existing auth.json file
pub fn import_from_auth_json(path: &str, account_name: String) -> Result<StoredAccount> {
    let content =
        fs::read_to_string(path).with_context(|| format!("Failed to read auth.json: {path}"))?;

    import_from_auth_json_contents(&content, account_name)
        .with_context(|| format!("Failed to parse auth.json: {path}"))
}

/// Import an account from auth.json file contents.
pub fn import_from_auth_json_contents(
    content: &str,
    account_name: String,
) -> Result<StoredAccount> {
    let auth: AuthDotJson =
        serde_json::from_str(&content).context("Failed to parse auth.json contents")?;
    let account_name = account_name.trim().to_string();

    // Determine auth mode and create account
    if let Some(api_key) = auth.openai_api_key {
        Ok(StoredAccount::new_api_key(account_name, api_key))
    } else if let Some(tokens) = auth.tokens {
        let claims = parse_chatgpt_id_token_claims(&tokens.id_token);

        Ok(StoredAccount::new_chatgpt(
            account_name,
            claims.email,
            claims.plan_type,
            claims.subscription_expires_at,
            tokens.id_token,
            tokens.access_token,
            tokens.refresh_token,
            claims.account_id.or(tokens.account_id),
        ))
    } else {
        anyhow::bail!("auth.json contains neither API key nor tokens");
    }
}

/// Read the current auth.json file if it exists
pub fn read_current_auth() -> Result<Option<AuthDotJson>> {
    let path = get_codex_auth_file()?;

    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read auth.json: {}", path.display()))?;

    let auth: AuthDotJson = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse auth.json: {}", path.display()))?;

    Ok(Some(auth))
}

/// Check if there is an active Codex login
pub fn has_active_login() -> Result<bool> {
    match read_current_auth()? {
        Some(auth) => Ok(auth.openai_api_key.is_some() || auth.tokens.is_some()),
        None => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        get_codex_auth_file, get_codex_home, import_from_auth_json_contents,
        set_codex_home_override, switch_to_account, validate_codex_home, wsl_distribution,
    };
    use crate::auth::{load_accounts, save_accounts, test_support::TestHome};
    use crate::types::{AccountsStore, StoredAccount};
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    use serde_json::json;

    #[test]
    fn changing_home_does_not_copy_auth_and_next_switch_targets_only_selected_home() {
        let _home = TestHome::new();
        let original = get_codex_home().unwrap();
        let account = StoredAccount::new_api_key("Selected".into(), "test-selected".into());
        switch_to_account(&account).unwrap();
        save_accounts(&AccountsStore {
            active_account_id: Some(account.id.clone()),
            accounts: vec![account.clone()],
            ..AccountsStore::default()
        })
        .unwrap();
        let original_bytes = std::fs::read(get_codex_auth_file().unwrap()).unwrap();
        let target = original.parent().unwrap().join("other-home");
        std::fs::create_dir(&target).unwrap();
        set_codex_home_override(Some(target.to_string_lossy().into_owned())).unwrap();
        assert_eq!(get_codex_home().unwrap(), target);
        assert!(!target.join("auth.json").exists());
        assert!(load_accounts().unwrap().active_account_id.is_none());
        switch_to_account(&account).unwrap();
        let target_bytes = std::fs::read(target.join("auth.json")).unwrap();
        assert_eq!(target_bytes, original_bytes);
        assert_eq!(
            std::fs::read(original.join("auth.json")).unwrap(),
            original_bytes
        );
        set_codex_home_override(None).unwrap();
        assert_eq!(get_codex_home().unwrap(), original);
        assert_eq!(
            std::fs::read(target.join("auth.json")).unwrap(),
            target_bytes
        );
    }

    #[test]
    fn invalid_home_does_not_change_settings_or_active_selection() {
        let _home = TestHome::new();
        let original = get_codex_home().unwrap();
        let account = StoredAccount::new_api_key("Selected".into(), "test-selected".into());
        save_accounts(&AccountsStore {
            active_account_id: Some(account.id.clone()),
            accounts: vec![account.clone()],
            ..AccountsStore::default()
        })
        .unwrap();
        for path in [
            "relative".to_string(),
            original.join("missing").to_string_lossy().into_owned(),
        ] {
            assert!(set_codex_home_override(Some(path)).is_err());
            assert_eq!(get_codex_home().unwrap(), original);
            assert_eq!(
                load_accounts().unwrap().active_account_id,
                Some(account.id.clone())
            );
        }
        assert!(validate_codex_home("").is_err());
        assert!(validate_codex_home("~/codex").is_err());
    }

    #[test]
    fn invalid_override_from_another_device_can_be_reset() {
        let _home = TestHome::new();
        let original = get_codex_home().unwrap();
        let mut settings = crate::auth::load_app_settings().unwrap();
        settings.codex_home_override = Some("invalid-relative-home".into());
        crate::auth::save_app_settings(&settings).unwrap();
        assert!(get_codex_home().is_err());
        set_codex_home_override(None).unwrap();
        assert_eq!(get_codex_home().unwrap(), original);
    }

    #[test]
    fn parses_only_local_wsl_unc_targets() {
        for path in [
            r"\\wsl.localhost\Ubuntu\home\test\.codex",
            r"\\WSL$\Ubuntu\home\test\.codex",
        ] {
            assert_eq!(wsl_distribution(path), Some("Ubuntu"));
        }
        for path in [
            r"C:\Users\Test\.codex",
            r"\\server\Ubuntu\home",
            r"\\wsl$\--exec\home",
            r"\\wsl$\..\home",
            r"\\wsl$\Ubuntu",
            r"\\wsl$\Ubuntu\",
            r"\\wsl$\Ubuntu;command\home",
        ] {
            assert_eq!(wsl_distribution(path), None);
        }
    }

    fn auth_json(payload: serde_json::Value, account_id: &str) -> String {
        let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap());
        serde_json::json!({
            "tokens": {
                "id_token": format!("header.{payload}.signature"),
                "access_token": "access",
                "refresh_token": "refresh",
                "account_id": account_id
            }
        })
        .to_string()
    }

    #[test]
    fn import_blank_name_uses_email() {
        let account = import_from_auth_json_contents(
            &auth_json(json!({"email": "imported@example.com"}), "acct-import"),
            "".into(),
        )
        .unwrap();
        assert_eq!(account.name, "imported@example.com");
    }

    #[test]
    fn import_explicit_name_is_trimmed() {
        let account = import_from_auth_json_contents(
            &auth_json(json!({"email": "imported@example.com"}), "acct-import"),
            "  Imported Account  ".into(),
        )
        .unwrap();
        assert_eq!(account.name, "Imported Account");
    }

    #[test]
    fn import_without_email_uses_account_id_fallback() {
        let account =
            import_from_auth_json_contents(&auth_json(json!({}), "acct-87654321"), "".into())
                .unwrap();
        assert_eq!(account.name, "ChatGPT account (87654321)");
    }
}
