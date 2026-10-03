use sha2::{Digest, Sha256};

#[cfg(all(target_os = "linux", not(feature = "key-chain-file")))]
mod linux;

/// Only an explicit login can initialize a missing desktop keyring. Passwords
/// are entered into the Secret Service's own prompt, never the CLI or launcher.
pub fn prepare_login() -> Result<(), String> {
    #[cfg(all(target_os = "linux", not(feature = "key-chain-file")))]
    linux::prepare(linux::Preparation::Login).map_err(|error| error.to_string())?;
    Ok(())
}

/// Unlock an existing desktop keyring before the service reads any credentials.
/// Unlike login, service startup never creates a missing keyring.
pub fn prepare_service() -> Result<(), String> {
    #[cfg(all(target_os = "linux", not(feature = "key-chain-file")))]
    linux::prepare(linux::Preparation::Service).map_err(|error| error.to_string())?;
    Ok(())
}

pub static SERVICE_NAME: &str = "Xodus Service";

pub fn init_secrets() -> Result<(), keyring_core::Error> {
    // Validate an explicit profile before touching credential storage.
    let profile = crate::config::directory()
        .map_err(|error| keyring_core::Error::NoStorageAccess(Box::new(error)))?;

    #[cfg(feature = "key-chain-file")]
    {
        let path = profile
            .map(|directory| directory.join(".xodus-keyring.ron"))
            .unwrap_or_else(secrets_backing_file);
        let store =
            keyring_core::sample::Store::new_with_backing(path.to_str().ok_or_else(|| {
                keyring_core::Error::Invalid(
                    "XODUS_CONFIG_DIR".into(),
                    "File-backed storage requires a UTF-8 path".into(),
                )
            })?)?;
        keyring_core::set_default_store(store);
    }

    #[cfg(not(feature = "key-chain-file"))]
    {
        let _ = profile;
        #[cfg(target_os = "linux")]
        {
            keyring_core::set_default_store(dbus_secret_service_keyring_store::Store::new()?);
        }

        #[cfg(target_os = "macos")]
        {
            keyring_core::set_default_store(apple_native_keyring_store::keychain::Store::new()?);
        }

        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let store = keyring_core::sample::Store::new_with_configuration(
                &std::collections::HashMap::from([("persist", "true")]),
            )?;
            keyring_core::set_default_store(store);
        }
    }

    Ok(())
}

pub fn get_entry(user: &str) -> Result<keyring_core::Entry, keyring_core::Error> {
    let profile = crate::config::directory()
        .map_err(|error| keyring_core::Error::NoStorageAccess(Box::new(error)))?;
    let service = match profile {
        Some(directory) => {
            let scope: String = Sha256::digest(directory.as_os_str().as_encoded_bytes())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            format!("{SERVICE_NAME}:{scope}")
        }
        None => SERVICE_NAME.to_string(),
    };
    keyring_core::Entry::new(&service, user)
}

pub fn destroy_secrets() {
    keyring_core::unset_default_store();
}

#[cfg(feature = "key-chain-file")]
fn secrets_backing_file() -> std::path::PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(".xodus-keyring.ron")
}
