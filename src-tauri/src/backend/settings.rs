use std::{
    fs,
    path::{Path, PathBuf},
};

use super::models::AppSettings;

const SETTINGS_FILE_NAME: &str = "app-settings.json";

pub fn load(app_data_directory: &Path) -> Result<(PathBuf, AppSettings), String> {
    fs::create_dir_all(app_data_directory)
        .map_err(|error| format!("Could not create FileDrop data directory: {error}"))?;
    let path = app_data_directory.join(SETTINGS_FILE_NAME);
    let settings = fs::read_to_string(&path)
        .ok()
        .and_then(|contents| serde_json::from_str(&contents).ok())
        .unwrap_or_default();
    persist(&path, &settings)?;
    Ok((path, settings))
}

pub fn persist(path: &Path, settings: &AppSettings) -> Result<(), String> {
    let encoded = serde_json::to_vec_pretty(settings)
        .map_err(|error| format!("Could not encode FileDrop settings: {error}"))?;
    fs::write(path, encoded).map_err(|error| format!("Could not save FileDrop settings: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn settings_default_and_persist_between_loads() {
        let directory = std::env::temp_dir().join(format!("filedrop-settings-{}", Uuid::new_v4()));
        let (path, first) = load(&directory).expect("default settings");
        assert!(first.discoverable);
        assert!(!first.auto_open_received);

        let changed = AppSettings {
            auto_open_received: true,
            discoverable: false,
        };
        persist(&path, &changed).expect("save settings");
        let (_, second) = load(&directory).expect("stored settings");
        assert_eq!(second, changed);
        let _ = fs::remove_dir_all(directory);
    }
}
