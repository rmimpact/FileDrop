use std::io::{Read, Write};

use serde::{de::DeserializeOwned, Deserialize, Serialize};

use super::models::FileMetadata;

const MAX_CONTROL_FRAME_BYTES: usize = 1024 * 1024;

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum OfferResponse {
    Accepted,
    Denied { message: String },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileHeader {
    pub transfer_id: String,
    pub file_index: usize,
    pub file: FileMetadata,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferComplete {
    pub transfer_id: String,
}

pub fn write_frame<W: Write, T: Serialize>(writer: &mut W, value: &T) -> Result<(), String> {
    let body = serde_json::to_vec(value)
        .map_err(|error| format!("Could not encode transfer message: {error}"))?;

    if body.len() > MAX_CONTROL_FRAME_BYTES {
        return Err("Transfer message is too large".to_string());
    }

    writer
        .write_all(&(body.len() as u32).to_be_bytes())
        .and_then(|_| writer.write_all(&body))
        .map_err(|error| format!("Could not send transfer message: {error}"))
}

pub fn read_frame<R: Read, T: DeserializeOwned>(reader: &mut R) -> Result<T, String> {
    let mut length_bytes = [0_u8; 4];
    reader
        .read_exact(&mut length_bytes)
        .map_err(|error| format!("Could not read transfer message: {error}"))?;

    let length = u32::from_be_bytes(length_bytes) as usize;
    if length == 0 || length > MAX_CONTROL_FRAME_BYTES {
        return Err("Transfer message has an invalid size".to_string());
    }

    let mut body = vec![0_u8; length];
    reader
        .read_exact(&mut body)
        .map_err(|error| format!("Could not read transfer message body: {error}"))?;

    serde_json::from_slice(&body)
        .map_err(|error| format!("Could not decode transfer message: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framed_messages_round_trip() {
        let original = TransferComplete {
            transfer_id: "transfer-123".to_string(),
        };
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &original).expect("encode frame");
        let decoded: TransferComplete = read_frame(&mut bytes.as_slice()).expect("decode frame");
        assert_eq!(decoded.transfer_id, original.transfer_id);
    }
}
