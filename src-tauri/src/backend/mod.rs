mod identity;
pub mod models;
mod protocol;
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

pub use models::{AppInfo, BackendEvent, NearbyDevice};
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
    emit: EventCallback,
    _mdns: ServiceDaemon,
}

impl Backend {
    pub fn start(
        app_data_directory: &Path,
        download_directory: PathBuf,
        emit: EventCallback,
    ) -> Result<Arc<Self>, String> {
        let identity = identity::load_or_create(app_data_directory)?;
        let listener = TcpListener::bind("0.0.0.0:0")
            .map_err(|error| format!("Could not start the FileDrop receiver: {error}"))?;
        let port = listener
            .local_addr()
            .map_err(|error| format!("Could not read the FileDrop receiver address: {error}"))?
            .port();
        let mdns = ServiceDaemon::new()
            .map_err(|error| format!("Could not start local-network discovery: {error}"))?;

        let backend = Arc::new(Self {
            identity,
            download_directory,
            devices: Mutex::new(HashMap::new()),
            pending_decisions: Mutex::new(HashMap::new()),
            cancellations: Mutex::new(HashMap::new()),
            emit,
            _mdns: mdns.clone(),
        });

        backend.register_service(&mdns, port)?;
        backend.start_discovery(&mdns)?;
        backend.start_listener(listener);
        Ok(backend)
    }

    pub fn app_info(&self) -> AppInfo {
        AppInfo {
            identity: self.identity.clone(),
            download_directory: self.download_directory.to_string_lossy().to_string(),
            protocol_version: PROTOCOL_VERSION,
        }
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

    fn register_service(&self, mdns: &ServiceDaemon, port: u16) -> Result<(), String> {
        let host_label = self.identity.id.chars().take(12).collect::<String>();
        let hostname = format!("filedrop-{host_label}.local.");
        let properties = [
            ("id", self.identity.id.as_str()),
            ("name", self.identity.name.as_str()),
            ("platform", self.identity.platform.as_str()),
            ("protocol", "1"),
        ];
        let service = ServiceInfo::new(
            SERVICE_TYPE,
            &self.identity.id,
            &hostname,
            "",
            port,
            &properties[..],
        )
        .map_err(|error| format!("Could not create the discovery announcement: {error}"))?
        .enable_addr_auto();

        mdns.register(service)
            .map_err(|error| format!("Could not announce FileDrop on the network: {error}"))
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
