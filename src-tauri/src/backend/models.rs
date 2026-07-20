use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u16 = 1;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceIdentity {
    pub id: String,
    pub name: String,
    pub platform: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NearbyDevice {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub address: String,
    pub port: u16,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub identity: DeviceIdentity,
    pub download_directory: String,
    pub protocol_version: u16,
    pub settings: AppSettings,
    pub network_online: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub auto_open_received: bool,
    pub discoverable: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            auto_open_received: false,
            discoverable: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectedFileInfo {
    pub path: String,
    pub name: String,
    pub size: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileMetadata {
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferOffer {
    pub protocol_version: u16,
    pub transfer_id: String,
    pub sender: DeviceIdentity,
    pub files: Vec<FileMetadata>,
    pub total_bytes: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TransferDirection {
    Sending,
    Receiving,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferProgress {
    pub transfer_id: String,
    pub direction: TransferDirection,
    pub current_file: String,
    pub current_file_index: usize,
    pub completed_files: usize,
    pub total_files: usize,
    pub transferred_bytes: u64,
    pub total_bytes: u64,
    pub remaining_bytes: u64,
    pub bytes_per_second: u64,
    pub progress: u8,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferFinished {
    pub transfer_id: String,
    pub direction: TransferDirection,
    pub saved_files: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferFailed {
    pub transfer_id: String,
    pub direction: TransferDirection,
    pub message: String,
}

#[derive(Clone, Debug)]
pub enum BackendEvent {
    DevicesChanged(Vec<NearbyDevice>),
    SettingsChanged(AppSettings),
    NetworkStatusChanged(bool),
    IncomingOffer(TransferOffer),
    TransferProgress(TransferProgress),
    TransferFinished(TransferFinished),
    TransferFailed(TransferFailed),
}
