use std::{fs, path::Path};

#[cfg(target_os = "macos")]
use std::process::Command;

use uuid::Uuid;

use super::models::DeviceIdentity;

const IDENTITY_FILE_NAME: &str = "device-identity.json";

pub fn load_or_create(app_data_dir: &Path) -> Result<DeviceIdentity, String> {
    fs::create_dir_all(app_data_dir)
        .map_err(|error| format!("Could not create FileDrop data directory: {error}"))?;

    let identity_path = app_data_dir.join(IDENTITY_FILE_NAME);
    let current_name = system_device_name();
    let current_platform = platform_name().to_string();

    if let Ok(contents) = fs::read_to_string(&identity_path) {
        if let Ok(mut identity) = serde_json::from_str::<DeviceIdentity>(&contents) {
            identity.name = current_name;
            identity.platform = current_platform;
            persist(&identity_path, &identity)?;
            return Ok(identity);
        }
    }

    let identity = DeviceIdentity {
        id: Uuid::new_v4().to_string(),
        name: current_name,
        platform: current_platform,
    };

    persist(&identity_path, &identity)?;
    Ok(identity)
}

fn persist(path: &Path, identity: &DeviceIdentity) -> Result<(), String> {
    let encoded = serde_json::to_vec_pretty(identity)
        .map_err(|error| format!("Could not encode device identity: {error}"))?;
    fs::write(path, encoded).map_err(|error| format!("Could not save device identity: {error}"))
}

fn system_device_name() -> String {
    platform_device_name()
        .or_else(environment_device_name)
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "FileDrop Device".to_string())
}

#[cfg(target_os = "macos")]
fn platform_device_name() -> Option<String> {
    let output = Command::new("scutil")
        .args(["--get", "ComputerName"])
        .output()
        .ok()?;

    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(target_os = "windows")]
fn platform_device_name() -> Option<String> {
    std::env::var("COMPUTERNAME").ok()
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn platform_device_name() -> Option<String> {
    None
}

fn environment_device_name() -> Option<String> {
    std::env::var("COMPUTERNAME")
        .ok()
        .or_else(|| std::env::var("HOSTNAME").ok())
}

fn platform_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "mac"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "other"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_stable_between_loads() {
        let directory = std::env::temp_dir().join(format!("filedrop-identity-{}", Uuid::new_v4()));
        let first = load_or_create(&directory).expect("first identity");
        let second = load_or_create(&directory).expect("second identity");

        assert_eq!(first.id, second.id);
        assert!(!second.name.is_empty());

        let _ = fs::remove_dir_all(directory);
    }
}
