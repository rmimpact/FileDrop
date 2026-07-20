use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{
    models::{
        BackendEvent, DeviceIdentity, FileMetadata, TransferDirection, TransferFailed,
        TransferFinished, TransferOffer, TransferProgress, PROTOCOL_VERSION,
    },
    protocol::{read_frame, write_frame, FileHeader, OfferResponse, TransferComplete},
};

const IO_BUFFER_BYTES: usize = 64 * 1024;
const MAX_FILES_PER_TRANSFER: usize = 512;

pub type EventCallback = Arc<dyn Fn(BackendEvent) + Send + Sync + 'static>;
pub type DecisionCallback = Arc<dyn Fn(&TransferOffer) -> IncomingDecision + Send + Sync + 'static>;

pub struct IncomingDecision {
    pub accepted: bool,
    pub cancellation: Arc<AtomicBool>,
}

pub fn new_cancellation() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

pub fn prepare_offer(
    sender: &DeviceIdentity,
    paths: &[PathBuf],
) -> Result<(TransferOffer, Vec<PathBuf>), String> {
    if paths.is_empty() {
        return Err("Select at least one file".to_string());
    }
    if paths.len() > MAX_FILES_PER_TRANSFER {
        return Err(format!(
            "A transfer can contain at most {MAX_FILES_PER_TRANSFER} files"
        ));
    }

    let mut files = Vec::with_capacity(paths.len());
    let mut validated_paths = Vec::with_capacity(paths.len());

    for path in paths {
        let metadata = fs::metadata(path)
            .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
        if !metadata.is_file() {
            return Err(format!("{} is not a file", path.display()));
        }

        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .ok_or_else(|| format!("{} does not have a valid file name", path.display()))?
            .to_string();

        files.push(FileMetadata {
            name,
            size: metadata.len(),
            sha256: hash_file(path)?,
        });
        validated_paths.push(path.clone());
    }

    let total_bytes = files.iter().map(|file| file.size).sum();
    Ok((
        TransferOffer {
            protocol_version: PROTOCOL_VERSION,
            transfer_id: Uuid::new_v4().to_string(),
            sender: sender.clone(),
            files,
            total_bytes,
        },
        validated_paths,
    ))
}

pub fn send_prepared_transfer(
    address: SocketAddr,
    offer: TransferOffer,
    paths: Vec<PathBuf>,
    cancellation: Arc<AtomicBool>,
    emit: EventCallback,
) -> Result<TransferFinished, TransferFailed> {
    let transfer_id = offer.transfer_id.clone();
    let result = send_inner(address, &offer, &paths, &cancellation, &emit);

    match result {
        Ok(()) => Ok(TransferFinished {
            transfer_id,
            direction: TransferDirection::Sending,
            saved_files: Vec::new(),
        }),
        Err(message) => Err(TransferFailed {
            transfer_id,
            direction: TransferDirection::Sending,
            message,
        }),
    }
}

fn send_inner(
    address: SocketAddr,
    offer: &TransferOffer,
    paths: &[PathBuf],
    cancellation: &AtomicBool,
    emit: &EventCallback,
) -> Result<(), String> {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(10))
        .map_err(|error| format!("Could not connect to the receiving device: {error}"))?;
    stream
        .set_nodelay(true)
        .map_err(|error| format!("Could not configure the transfer connection: {error}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(300)))
        .map_err(|error| format!("Could not configure the transfer timeout: {error}"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(60)))
        .map_err(|error| format!("Could not configure the transfer timeout: {error}"))?;

    write_frame(&mut stream, offer)?;
    match read_frame::<_, OfferResponse>(&mut stream)? {
        OfferResponse::Accepted => {}
        OfferResponse::Denied { message } => return Err(message),
    }

    let mut transferred_bytes = 0_u64;
    let mut buffer = vec![0_u8; IO_BUFFER_BYTES];
    let started_at = Instant::now();

    for (file_index, (path, file_metadata)) in paths.iter().zip(&offer.files).enumerate() {
        ensure_not_cancelled(cancellation)?;
        write_frame(
            &mut stream,
            &FileHeader {
                transfer_id: offer.transfer_id.clone(),
                file_index,
                file: file_metadata.clone(),
            },
        )?;

        let mut file = File::open(path)
            .map_err(|error| format!("Could not open {}: {error}", path.display()))?;
        let mut file_bytes = 0_u64;

        loop {
            ensure_not_cancelled(cancellation)?;
            let read = file
                .read(&mut buffer)
                .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
            if read == 0 {
                break;
            }

            stream
                .write_all(&buffer[..read])
                .map_err(|error| format!("The network transfer was interrupted: {error}"))?;
            file_bytes += read as u64;
            transferred_bytes += read as u64;
            emit_progress(
                emit,
                offer,
                TransferDirection::Sending,
                file_index,
                transferred_bytes,
                started_at,
            );
        }

        if file_bytes != file_metadata.size {
            return Err(format!(
                "{} changed while it was being sent",
                file_metadata.name
            ));
        }
    }

    write_frame(
        &mut stream,
        &TransferComplete {
            transfer_id: offer.transfer_id.clone(),
        },
    )?;
    Ok(())
}

pub fn receive_transfer(
    mut stream: TcpStream,
    destination_directory: &Path,
    decide: DecisionCallback,
    emit: EventCallback,
) -> Result<TransferFinished, TransferFailed> {
    if let Err(error) = stream.set_read_timeout(Some(Duration::from_secs(300))) {
        return Err(TransferFailed {
            transfer_id: "unknown".to_string(),
            direction: TransferDirection::Receiving,
            message: format!("Could not configure the transfer timeout: {error}"),
        });
    }
    if let Err(error) = stream.set_write_timeout(Some(Duration::from_secs(60))) {
        return Err(TransferFailed {
            transfer_id: "unknown".to_string(),
            direction: TransferDirection::Receiving,
            message: format!("Could not configure the transfer timeout: {error}"),
        });
    }

    let offer = match read_frame::<_, TransferOffer>(&mut stream) {
        Ok(offer) => offer,
        Err(message) => {
            return Err(TransferFailed {
                transfer_id: "unknown".to_string(),
                direction: TransferDirection::Receiving,
                message,
            })
        }
    };
    let transfer_id = offer.transfer_id.clone();

    let result = receive_inner(&mut stream, destination_directory, &offer, decide, &emit);

    match result {
        Ok(saved_files) => Ok(TransferFinished {
            transfer_id,
            direction: TransferDirection::Receiving,
            saved_files,
        }),
        Err(message) => Err(TransferFailed {
            transfer_id,
            direction: TransferDirection::Receiving,
            message,
        }),
    }
}

fn receive_inner(
    stream: &mut TcpStream,
    destination_directory: &Path,
    offer: &TransferOffer,
    decide: DecisionCallback,
    emit: &EventCallback,
) -> Result<Vec<String>, String> {
    validate_offer(offer)?;
    let decision = decide(offer);

    if !decision.accepted {
        write_frame(
            stream,
            &OfferResponse::Denied {
                message: "The receiving device denied the transfer".to_string(),
            },
        )?;
        return Err("Transfer denied".to_string());
    }

    fs::create_dir_all(destination_directory).map_err(|error| {
        format!(
            "Could not create the download directory {}: {error}",
            destination_directory.display()
        )
    })?;
    write_frame(stream, &OfferResponse::Accepted)?;

    let mut reserved_destinations = HashSet::new();
    let planned_files = offer
        .files
        .iter()
        .enumerate()
        .map(|(index, file)| {
            let destination = unique_destination(
                destination_directory,
                &file.name,
                &mut reserved_destinations,
            )?;
            let temporary =
                destination_directory.join(format!(".filedrop-{}-{index}.part", offer.transfer_id));
            Ok((temporary, destination))
        })
        .collect::<Result<Vec<_>, String>>()?;

    let receive_result =
        receive_file_contents(stream, offer, &planned_files, &decision.cancellation, emit);

    if let Err(message) = receive_result {
        cleanup_temporary_files(&planned_files);
        return Err(message);
    }

    let completed: TransferComplete = read_frame(stream)?;
    if completed.transfer_id != offer.transfer_id {
        cleanup_temporary_files(&planned_files);
        return Err("The transfer completion message did not match the offer".to_string());
    }

    let mut saved_files = Vec::with_capacity(planned_files.len());
    for (temporary, destination) in &planned_files {
        fs::rename(temporary, destination).map_err(|error| {
            format!("Could not finish saving {}: {error}", destination.display())
        })?;
        saved_files.push(destination.to_string_lossy().to_string());
    }

    Ok(saved_files)
}

fn receive_file_contents(
    stream: &mut TcpStream,
    offer: &TransferOffer,
    planned_files: &[(PathBuf, PathBuf)],
    cancellation: &AtomicBool,
    emit: &EventCallback,
) -> Result<(), String> {
    let mut transferred_bytes = 0_u64;
    let mut buffer = vec![0_u8; IO_BUFFER_BYTES];
    let started_at = Instant::now();

    for (file_index, (temporary, _)) in planned_files.iter().enumerate() {
        ensure_not_cancelled(cancellation)?;
        let header: FileHeader = read_frame(stream)?;
        let expected = &offer.files[file_index];

        if header.transfer_id != offer.transfer_id
            || header.file_index != file_index
            || header.file != *expected
        {
            return Err("The sender provided unexpected file metadata".to_string());
        }

        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(temporary)
            .map_err(|error| format!("Could not create a temporary download file: {error}"))?;
        let mut hasher = Sha256::new();
        let mut remaining = expected.size;

        while remaining > 0 {
            ensure_not_cancelled(cancellation)?;
            let wanted = remaining.min(buffer.len() as u64) as usize;
            let read = stream
                .read(&mut buffer[..wanted])
                .map_err(|error| format!("The network transfer was interrupted: {error}"))?;
            if read == 0 {
                return Err("The sender disconnected before the transfer completed".to_string());
            }

            output
                .write_all(&buffer[..read])
                .map_err(|error| format!("Could not write the received file: {error}"))?;
            hasher.update(&buffer[..read]);
            remaining -= read as u64;
            transferred_bytes += read as u64;
            emit_progress(
                emit,
                offer,
                TransferDirection::Receiving,
                file_index,
                transferred_bytes,
                started_at,
            );
        }

        output
            .sync_all()
            .map_err(|error| format!("Could not finish writing the received file: {error}"))?;
        let actual_hash = format!("{:x}", hasher.finalize());
        if actual_hash != expected.sha256 {
            return Err(format!("{} failed its integrity check", expected.name));
        }
    }

    Ok(())
}

fn validate_offer(offer: &TransferOffer) -> Result<(), String> {
    if offer.protocol_version != PROTOCOL_VERSION {
        return Err(format!(
            "This device uses FileDrop protocol {}, but the sender uses protocol {}",
            PROTOCOL_VERSION, offer.protocol_version
        ));
    }
    if offer.files.is_empty() || offer.files.len() > MAX_FILES_PER_TRANSFER {
        return Err("The transfer contains an invalid number of files".to_string());
    }
    if offer.sender.id.is_empty() || offer.sender.name.trim().is_empty() {
        return Err("The sender identity is invalid".to_string());
    }

    let calculated_total = offer.files.iter().try_fold(0_u64, |total, file| {
        validate_file_name(&file.name)?;
        if file.sha256.len() != 64 || !file.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(format!("{} has an invalid integrity hash", file.name));
        }
        total
            .checked_add(file.size)
            .ok_or_else(|| "The transfer size is too large".to_string())
    })?;

    if calculated_total != offer.total_bytes {
        return Err("The transfer size does not match its file list".to_string());
    }
    Ok(())
}

fn validate_file_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
    {
        return Err("The transfer contains an unsafe file name".to_string());
    }
    Ok(())
}

fn unique_destination(
    directory: &Path,
    file_name: &str,
    reserved: &mut HashSet<PathBuf>,
) -> Result<PathBuf, String> {
    let portable_name = portable_file_name(file_name)?;
    let original = Path::new(&portable_name);
    let stem = original
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("File");
    let extension = original
        .extension()
        .and_then(|extension| extension.to_str());

    for copy_number in 0..10_000_u32 {
        let candidate_name = if copy_number == 0 {
            portable_name.clone()
        } else if let Some(extension) = extension {
            format!("{stem} ({copy_number}).{extension}")
        } else {
            format!("{stem} ({copy_number})")
        };
        let candidate = directory.join(candidate_name);
        if !candidate.exists() && reserved.insert(candidate.clone()) {
            return Ok(candidate);
        }
    }

    Err(format!(
        "Could not choose a safe destination for {file_name}"
    ))
}

fn portable_file_name(file_name: &str) -> Result<String, String> {
    validate_file_name(file_name)?;
    let mut sanitized = file_name
        .chars()
        .map(|character| {
            if character.is_control()
                || matches!(character, '<' | '>' | ':' | '"' | '|' | '?' | '*')
            {
                '_'
            } else {
                character
            }
        })
        .collect::<String>();

    while sanitized.ends_with('.') || sanitized.ends_with(' ') {
        sanitized.pop();
    }
    if sanitized.is_empty() {
        sanitized.push_str("File");
    }

    let stem = Path::new(&sanitized)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default()
        .to_ascii_uppercase();
    let reserved_windows_name = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit()
            && stem.as_bytes()[3] != b'0');
    if reserved_windows_name {
        sanitized.insert(0, '_');
    }

    Ok(sanitized)
}

fn cleanup_temporary_files(planned_files: &[(PathBuf, PathBuf)]) {
    for (temporary, _) in planned_files {
        let _ = fs::remove_file(temporary);
    }
}

fn hash_file(path: &Path) -> Result<String, String> {
    let mut file =
        File::open(path).map_err(|error| format!("Could not open {}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; IO_BUFFER_BYTES];

    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(format!("{:x}", hasher.finalize()))
}

fn ensure_not_cancelled(cancellation: &AtomicBool) -> Result<(), String> {
    if cancellation.load(Ordering::Relaxed) {
        Err("Transfer cancelled".to_string())
    } else {
        Ok(())
    }
}

fn emit_progress(
    emit: &EventCallback,
    offer: &TransferOffer,
    direction: TransferDirection,
    current_file_index: usize,
    transferred_bytes: u64,
    started_at: Instant,
) {
    let progress = transferred_bytes
        .saturating_mul(100)
        .checked_div(offer.total_bytes)
        .unwrap_or(100)
        .min(100) as u8;

    emit(BackendEvent::TransferProgress(TransferProgress {
        transfer_id: offer.transfer_id.clone(),
        direction,
        current_file: offer.files[current_file_index].name.clone(),
        current_file_index,
        completed_files: current_file_index,
        total_files: offer.files.len(),
        transferred_bytes,
        total_bytes: offer.total_bytes,
        remaining_bytes: offer.total_bytes.saturating_sub(transferred_bytes),
        bytes_per_second: if started_at.elapsed() < Duration::from_millis(100) {
            0
        } else {
            (transferred_bytes as f64 / started_at.elapsed().as_secs_f64()) as u64
        },
        progress,
    }));
}

#[cfg(test)]
mod tests {
    use std::{net::TcpListener, sync::Mutex, thread};

    use super::*;

    fn test_identity(name: &str) -> DeviceIdentity {
        DeviceIdentity {
            id: Uuid::new_v4().to_string(),
            name: name.to_string(),
            platform: "test".to_string(),
        }
    }

    #[test]
    fn two_virtual_devices_transfer_files_over_loopback() {
        let root = std::env::temp_dir().join(format!("filedrop-transfer-{}", Uuid::new_v4()));
        let source = root.join("source");
        let destination = root.join("destination");
        fs::create_dir_all(&source).expect("source directory");
        fs::write(source.join("hello.txt"), b"hello from virtual mac").expect("first file");
        fs::write(source.join("data.bin"), (0_u8..=255).collect::<Vec<_>>()).expect("second file");

        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback listener");
        let address = listener.local_addr().expect("listener address");
        let receiver_events = Arc::new(Mutex::new(Vec::new()));
        let receiver_event_copy = Arc::clone(&receiver_events);
        let receiver_emit: EventCallback = Arc::new(move |event| {
            receiver_event_copy.lock().expect("events lock").push(event);
        });
        let destination_copy = destination.clone();

        let receiver = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept sender");
            receive_transfer(
                stream,
                &destination_copy,
                Arc::new(|_| IncomingDecision {
                    accepted: true,
                    cancellation: new_cancellation(),
                }),
                receiver_emit,
            )
            .expect("receive transfer")
        });

        let paths = vec![source.join("hello.txt"), source.join("data.bin")];
        let (offer, validated_paths) =
            prepare_offer(&test_identity("Virtual Mac"), &paths).expect("prepare offer");
        let sender_result = send_prepared_transfer(
            address,
            offer,
            validated_paths,
            new_cancellation(),
            Arc::new(|_| {}),
        )
        .expect("send transfer");
        let receiver_result = receiver.join().expect("receiver thread");

        assert_eq!(sender_result.direction, TransferDirection::Sending);
        assert_eq!(receiver_result.saved_files.len(), 2);
        assert_eq!(
            fs::read(destination.join("hello.txt")).expect("received first file"),
            b"hello from virtual mac"
        );
        assert_eq!(
            fs::read(destination.join("data.bin")).expect("received second file"),
            (0_u8..=255).collect::<Vec<_>>()
        );
        let events = receiver_events.lock().expect("events lock");
        let final_progress = events
            .iter()
            .filter_map(|event| match event {
                BackendEvent::TransferProgress(progress) => Some(progress),
                _ => None,
            })
            .next_back()
            .expect("transfer progress event");
        assert_eq!(final_progress.progress, 100);
        assert_eq!(final_progress.transferred_bytes, final_progress.total_bytes);
        assert_eq!(final_progress.remaining_bytes, 0);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn duplicate_names_are_not_overwritten() {
        let directory = std::env::temp_dir().join(format!("filedrop-name-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).expect("directory");
        fs::write(directory.join("photo.png"), b"existing").expect("existing file");
        let mut reserved = HashSet::new();

        let first = unique_destination(&directory, "photo.png", &mut reserved).expect("first");
        let second = unique_destination(&directory, "photo.png", &mut reserved).expect("second");

        assert_eq!(first.file_name().unwrap(), "photo (1).png");
        assert_eq!(second.file_name().unwrap(), "photo (2).png");
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn unsafe_file_names_are_rejected() {
        assert!(validate_file_name("../secret.txt").is_err());
        assert!(validate_file_name("folder\\secret.txt").is_err());
        assert!(validate_file_name("safe file.txt").is_ok());
    }

    #[test]
    fn file_names_are_portable_to_windows() {
        assert_eq!(
            portable_file_name("report:final?.txt").unwrap(),
            "report_final_.txt"
        );
        assert_eq!(portable_file_name("CON.txt").unwrap(), "_CON.txt");
        assert_eq!(portable_file_name("photo. ").unwrap(), "photo");
    }
}
