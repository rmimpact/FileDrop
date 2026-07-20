mod identity;
pub mod models;
mod protocol;
mod settings;
mod transfer;

use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr, TcpListener},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::Duration,
};

use mdns_sd::{ResolvedService, ServiceDaemon, ServiceEvent, ServiceInfo};

pub use models::{AppInfo, AppSettings, BackendEvent, NearbyDevice, SelectedFileInfo};
use models::{DeviceIdentity, TransferFailed, TransferFinished, PROTOCOL_VERSION};
use transfer::{
    new_cancellation, prepare_offer, receive_transfer, send_prepared_transfer, DecisionCallback,
    EventCallback, IncomingDecision,
};

const SERVICE_TYPE: &str = "_filedrop._tcp.local.";

pub struct Backend {
    identity: DeviceIdentity,
    download_directory: PathBuf,
    devices: Mutex<HashMap<String, NearbyDevice>>,
    pending_decisions: Mutex<HashMap<String, mpsc::Sender<bool>>>,
    cancellations: Mutex<HashMap<String, Arc<AtomicBool>>>,
    settings_path: PathBuf,
    settings: Mutex<AppSettings>,
    network_online: AtomicBool,
    emit: EventCallback,
    mdns: ServiceDaemon,
    service: ServiceInfo,
}

impl Backend {
    pub fn start(
        app_data_directory: &Path,
        download_directory: PathBuf,
        emit: EventCallback,
    ) -> Result<Arc<Self>, String> {
        let identity = identity::load_or_create(app_data_directory)?;
        let (settings_path, settings) = settings::load(app_data_directory)?;
        let listener = TcpListener::bind("0.0.0.0:0")
            .map_err(|error| format!("Could not start the FileDrop receiver: {error}"))?;
        let port = listener
            .local_addr()
            .map_err(|error| format!("Could not read the FileDrop receiver address: {error}"))?
            .port();
        let mdns = ServiceDaemon::new()
            .map_err(|error| format!("Could not start local-network discovery: {error}"))?;
        let service = Self::service_info(&identity, port)?;
        let network_online = network_is_online();

        let backend = Arc::new(Self {
            identity,
            download_directory,
            devices: Mutex::new(HashMap::new()),
            pending_decisions: Mutex::new(HashMap::new()),
            cancellations: Mutex::new(HashMap::new()),
            settings_path,
            settings: Mutex::new(settings.clone()),
            network_online: AtomicBool::new(network_online),
            emit,
            mdns: mdns.clone(),
            service,
        });

        if settings.discoverable && network_online {
            backend.register_service()?;
        }
        backend.start_discovery(&mdns)?;
        backend.start_listener(listener);
        backend.start_network_monitor();
        Ok(backend)
    }

    pub fn app_info(&self) -> AppInfo {
        AppInfo {
            identity: self.identity.clone(),
            download_directory: self.download_directory.to_string_lossy().to_string(),
            protocol_version: PROTOCOL_VERSION,
            settings: self.settings(),
            network_online: self.network_online.load(Ordering::Relaxed),
        }
    }

    pub fn settings(&self) -> AppSettings {
        self.settings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn set_discoverable(&self, enabled: bool) -> Result<AppSettings, String> {
        let updated = self.update_settings(|settings| settings.discoverable = enabled)?;
        if enabled && self.network_online.load(Ordering::Relaxed) {
            self.register_service()?;
        } else if !enabled {
            self.unregister_service();
        }
        Ok(updated)
    }

    pub fn set_auto_open_received(&self, enabled: bool) -> Result<AppSettings, String> {
        self.update_settings(|settings| settings.auto_open_received = enabled)
    }

    pub fn inspect_files(&self, paths: Vec<PathBuf>) -> Result<Vec<SelectedFileInfo>, String> {
        paths
            .into_iter()
            .map(|path| {
                let metadata = std::fs::metadata(&path)
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
                Ok(SelectedFileInfo {
                    path: path.to_string_lossy().to_string(),
                    name,
                    size: metadata.len(),
                })
            })
            .collect()
    }

    pub fn nearby_devices(&self) -> Vec<NearbyDevice> {
        let mut devices = self
            .devices
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .cloned()
            .collect::<Vec<_>>();
        devices.sort_by_key(|device| device.name.to_lowercase());
        devices
    }

    pub fn send_files(
        self: &Arc<Self>,
        device_id: &str,
        paths: Vec<PathBuf>,
    ) -> Result<String, String> {
        let device = self
            .devices
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(device_id)
            .cloned()
            .ok_or_else(|| "That device is no longer available".to_string())?;
        let ip_address = device
            .address
            .parse::<IpAddr>()
            .map_err(|_| "The selected device has an invalid network address".to_string())?;
        let address = SocketAddr::new(ip_address, device.port);
        let (offer, validated_paths) = prepare_offer(&self.identity, &paths)?;
        let transfer_id = offer.transfer_id.clone();
        let cancellation = new_cancellation();

        self.cancellations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(transfer_id.clone(), Arc::clone(&cancellation));

        let backend = Arc::clone(self);
        thread::spawn(move || {
            let result = send_prepared_transfer(
                address,
                offer,
                validated_paths,
                cancellation,
                Arc::clone(&backend.emit),
            );
            backend.finish_transfer(result);
        });

        Ok(transfer_id)
    }

    pub fn respond_to_offer(&self, transfer_id: &str, accepted: bool) -> Result<(), String> {
        let sender = self
            .pending_decisions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(transfer_id)
            .ok_or_else(|| "That transfer request is no longer waiting".to_string())?;
        sender
            .send(accepted)
            .map_err(|_| "The sending device disconnected".to_string())
    }

    pub fn cancel_transfer(&self, transfer_id: &str) -> Result<(), String> {
        let cancellation = self
            .cancellations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(transfer_id)
            .cloned()
            .ok_or_else(|| "That transfer is no longer active".to_string())?;
        cancellation.store(true, Ordering::Relaxed);
        Ok(())
    }

    fn service_info(identity: &DeviceIdentity, port: u16) -> Result<ServiceInfo, String> {
        let host_label = identity.id.chars().take(12).collect::<String>();
        let hostname = format!("filedrop-{host_label}.local.");
        let properties = [
            ("id", identity.id.as_str()),
            ("name", identity.name.as_str()),
            ("platform", identity.platform.as_str()),
            ("protocol", "1"),
        ];
        ServiceInfo::new(
            SERVICE_TYPE,
            &identity.id,
            &hostname,
            "",
            port,
            &properties[..],
        )
        .map_err(|error| format!("Could not create the discovery announcement: {error}"))
        .map(ServiceInfo::enable_addr_auto)
    }

    fn register_service(&self) -> Result<(), String> {
        self.mdns
            .register(self.service.clone())
            .map_err(|error| format!("Could not announce FileDrop on the network: {error}"))
    }

    fn unregister_service(&self) {
        let _ = self.mdns.unregister(self.service.get_fullname());
    }

    fn update_settings(
        &self,
        update: impl FnOnce(&mut AppSettings),
    ) -> Result<AppSettings, String> {
        let updated = {
            let mut current = self
                .settings
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            update(&mut current);
            settings::persist(&self.settings_path, &current)?;
            current.clone()
        };
        (self.emit)(BackendEvent::SettingsChanged(updated.clone()));
        Ok(updated)
    }

    fn start_network_monitor(self: &Arc<Self>) {
        let backend = Arc::clone(self);
        thread::spawn(move || loop {
            thread::sleep(Duration::from_secs(3));
            let online = network_is_online();
            let previous = backend.network_online.swap(online, Ordering::Relaxed);
            if online == previous {
                continue;
            }

            if online && backend.settings().discoverable {
                let _ = backend.register_service();
            } else if !online {
                backend.unregister_service();
                backend
                    .devices
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .clear();
                (backend.emit)(BackendEvent::DevicesChanged(Vec::new()));
            }
            (backend.emit)(BackendEvent::NetworkStatusChanged(online));
        });
    }

    fn start_discovery(self: &Arc<Self>, mdns: &ServiceDaemon) -> Result<(), String> {
        let receiver = mdns
            .browse(SERVICE_TYPE)
            .map_err(|error| format!("Could not browse for FileDrop devices: {error}"))?;
        let backend = Arc::clone(self);

        thread::spawn(move || {
            while let Ok(event) = receiver.recv() {
                match event {
                    ServiceEvent::ServiceResolved(service) => {
                        backend.device_resolved(&service);
                    }
                    ServiceEvent::ServiceRemoved(_, fullname) => {
                        backend.device_removed(&fullname);
                    }
                    _ => {}
                }
            }
        });
        Ok(())
    }

    fn start_listener(self: &Arc<Self>, listener: TcpListener) {
        let backend = Arc::clone(self);
        thread::spawn(move || {
            for incoming in listener.incoming() {
                let Ok(stream) = incoming else {
                    continue;
                };
                let connection_backend = Arc::clone(&backend);
                thread::spawn(move || connection_backend.handle_incoming(stream));
            }
        });
    }

    fn handle_incoming(self: Arc<Self>, stream: std::net::TcpStream) {
        let decision_backend = Arc::clone(&self);
        let decide: DecisionCallback = Arc::new(move |offer| {
            let cancellation = new_cancellation();
            if !decision_backend.settings().discoverable
                || !decision_backend.network_online.load(Ordering::Relaxed)
            {
                return IncomingDecision {
                    accepted: false,
                    cancellation,
                };
            }
            let (decision_sender, decision_receiver) = mpsc::channel();
            decision_backend
                .cancellations
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(offer.transfer_id.clone(), Arc::clone(&cancellation));
            decision_backend
                .pending_decisions
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(offer.transfer_id.clone(), decision_sender);
            (decision_backend.emit)(BackendEvent::IncomingOffer(offer.clone()));

            let accepted = decision_receiver
                .recv_timeout(Duration::from_secs(300))
                .unwrap_or(false);
            decision_backend
                .pending_decisions
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&offer.transfer_id);
            IncomingDecision {
                accepted,
                cancellation,
            }
        });

        let result = receive_transfer(
            stream,
            &self.download_directory,
            decide,
            Arc::clone(&self.emit),
        );
        self.finish_transfer(result);
    }

    fn finish_transfer(&self, result: Result<TransferFinished, TransferFailed>) {
        let transfer_id = match &result {
            Ok(finished) => &finished.transfer_id,
            Err(failed) => &failed.transfer_id,
        }
        .clone();

        self.cancellations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&transfer_id);
        self.pending_decisions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&transfer_id);

        match result {
            Ok(finished) => (self.emit)(BackendEvent::TransferFinished(finished)),
            Err(failed) if failed.message == "Transfer denied" => {}
            Err(failed) => (self.emit)(BackendEvent::TransferFailed(failed)),
        }
    }

    fn device_resolved(&self, service: &ResolvedService) {
        let Some(id) = service.get_property_val_str("id") else {
            return;
        };
        if id == self.identity.id {
            return;
        }
        if service.get_property_val_str("protocol") != Some("1") {
            return;
        }
        let Some(name) = service.get_property_val_str("name") else {
            return;
        };
        let Some(address) = service
            .get_addresses()
            .iter()
            .find(|address| address.is_ipv4() && !address.is_loopback())
            .map(|address| address.to_ip_addr())
        else {
            return;
        };

        let device = NearbyDevice {
            id: id.to_string(),
            name: name.to_string(),
            platform: service
                .get_property_val_str("platform")
                .unwrap_or("other")
                .to_string(),
            address: address.to_string(),
            port: service.get_port(),
        };

        self.devices
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(device.id.clone(), device);
        (self.emit)(BackendEvent::DevicesChanged(self.nearby_devices()));
    }

    fn device_removed(&self, fullname: &str) {
        let id = fullname
            .strip_suffix(SERVICE_TYPE)
            .unwrap_or(fullname)
            .trim_end_matches('.');
        let removed = self
            .devices
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(id)
            .is_some();
        if removed {
            (self.emit)(BackendEvent::DevicesChanged(self.nearby_devices()));
        }
    }
}

fn network_is_online() -> bool {
    if_addrs::get_if_addrs().is_ok_and(|interfaces| {
        interfaces.into_iter().any(|interface| {
            interface.is_oper_up()
                && !interface.is_loopback()
                && !interface.is_link_local()
                && !interface.ip().is_unspecified()
        })
    })
}
