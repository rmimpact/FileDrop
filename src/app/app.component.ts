import { Component, NgZone, OnDestroy, OnInit } from '@angular/core';
import { NgTemplateOutlet } from '@angular/common';
import { invoke } from '@tauri-apps/api/core';
import { getVersion } from '@tauri-apps/api/app';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { open } from '@tauri-apps/plugin-dialog';
import { revealItemInDir } from '@tauri-apps/plugin-opener';
import {
  AppUpdateService,
  AvailableAppUpdate,
} from './app-update.service';

type DeviceType = 'mac' | 'windows' | 'other';
type TransferDirection = 'sending' | 'receiving';
type UpdateStatus = 'idle' | 'checking' | 'available' | 'downloading' | 'installing' | 'up-to-date' | 'error';

interface DeviceIdentity {
  id: string;
  name: string;
  platform: DeviceType;
}

interface NearbyDevice {
  id: string;
  name: string;
  platform: DeviceType;
  address: string;
  port: number;
}

interface AppInfo {
  identity: DeviceIdentity;
  downloadDirectory: string;
  protocolVersion: number;
  settings: AppSettings;
  networkOnline: boolean;
}

interface AppSettings {
  autoOpenReceived: boolean;
  discoverable: boolean;
}

interface SelectedFile {
  name: string;
  path: string;
  size: number;
}

interface TransferFile {
  name: string;
  size: number;
  sha256: string;
}

interface TransferOffer {
  protocolVersion: number;
  transferId: string;
  sender: DeviceIdentity;
  files: TransferFile[];
  totalBytes: number;
}

interface TransferProgress {
  transferId: string;
  direction: TransferDirection;
  currentFile: string;
  currentFileIndex: number;
  completedFiles: number;
  totalFiles: number;
  transferredBytes: number;
  totalBytes: number;
  remainingBytes: number;
  bytesPerSecond: number;
  progress: number;
}

interface TransferFinished {
  transferId: string;
  direction: TransferDirection;
  savedFiles: string[];
}

interface TransferFailed {
  transferId: string;
  direction: TransferDirection;
  message: string;
}

@Component({
  selector: 'app-root',
  standalone: true,
  imports: [NgTemplateOutlet],
  templateUrl: './app.component.html',
  styleUrl: './app.component.css',
})
export class AppComponent implements OnInit, OnDestroy {
  localDeviceName = 'This Device';
  localPlatform: DeviceType = 'other';
  appVersion = '1.1.1';
  downloadDirectory = '';
  settingsOpen = false;
  settingsClosing = false;
  selectedTheme: 'auto' | 'light' | 'dark' = 'auto';
  autoOpenReceived = false;
  discoverable = true;
  networkOnline = true;
  updateStatus: UpdateStatus = 'idle';
  availableUpdate: AvailableAppUpdate | null = null;
  updatePromptOpen = false;
  updateDownloadedBytes = 0;
  updateTotalBytes = 0;
  updateError = '';

  fileSelectionOpen = false;
  sendingOpen = false;
  receivingConfirmationOpen = false;
  receivingOpen = false;

  selectedDevice: NearbyDevice | null = null;
  selectedFiles: SelectedFile[] = [];
  devices: NearbyDevice[] = [];
  incomingOffer: TransferOffer | null = null;

  activeTransferId: string | null = null;
  transferProgress = 0;
  completedFiles = 0;
  currentFileIndex = 0;
  transferFinished = false;
  transferError = '';
  transferTransferredBytes = 0;
  transferTotalBytes = 0;
  transferRemainingBytes = 0;
  transferBytesPerSecond = 0;
  receivedSavedFiles: string[] = [];

  private unlistenFunctions: UnlistenFn[] = [];
  private readonly runningInTauri = '__TAURI_INTERNALS__' in window;
  private sendRequestGeneration = 0;
  private settingsCloseTimer: ReturnType<typeof setTimeout> | null = null;

  constructor(
    private readonly zone: NgZone,
    private readonly appUpdates: AppUpdateService,
  ) {}

  async ngOnInit() {
    if (!this.runningInTauri) {
      return;
    }

    await this.registerBackendListeners();

    try {
      const [appInfo, devices, appVersion] = await Promise.all([
        invoke<AppInfo>('get_app_info'),
        invoke<NearbyDevice[]>('get_nearby_devices'),
        getVersion(),
      ]);
      this.localDeviceName = appInfo.identity.name;
      this.localPlatform = appInfo.identity.platform;
      this.downloadDirectory = appInfo.downloadDirectory;
      this.autoOpenReceived = appInfo.settings.autoOpenReceived;
      this.discoverable = appInfo.settings.discoverable;
      this.networkOnline = appInfo.networkOnline;
      this.devices = devices;
      this.appVersion = appVersion;

      const unlistenDragDrop = await getCurrentWebview().onDragDropEvent((event) => {
        if (event.payload.type === 'drop' && this.fileSelectionOpen) {
          const paths = event.payload.paths;
          this.zone.run(() => void this.addFilePaths(paths));
        }
      });
      this.unlistenFunctions.push(unlistenDragDrop);

      // A failed background check stays silent; users can always retry from Settings.
      void this.checkForUpdates(false);
    } catch (error) {
      this.transferError = this.errorMessage(error);
    }
  }

  ngOnDestroy() {
    if (this.settingsCloseTimer) {
      clearTimeout(this.settingsCloseTimer);
    }

    for (const unlisten of this.unlistenFunctions) {
      unlisten();
    }

    if (this.updateStatus !== 'downloading' && this.updateStatus !== 'installing') {
      void this.appUpdates.dispose();
    }
  }

  openSettings() {
    if (this.settingsCloseTimer) {
      clearTimeout(this.settingsCloseTimer);
      this.settingsCloseTimer = null;
    }
    this.settingsClosing = false;
    this.settingsOpen = true;
  }

  closeSettings() {
    if (!this.settingsOpen || this.settingsClosing) {
      return;
    }

    this.settingsClosing = true;
    this.settingsCloseTimer = setTimeout(() => {
      this.settingsOpen = false;
      this.settingsClosing = false;
      this.settingsCloseTimer = null;
    }, 170);
  }

  setTheme(theme: 'auto' | 'light' | 'dark') {
    this.selectedTheme = theme;
  }

  async checkForUpdates(manual = true) {
    if (!this.runningInTauri || !this.desktopUpdatesSupported || this.updateStatus === 'checking' || this.updateStatus === 'downloading' || this.updateStatus === 'installing') {
      return;
    }

    if (this.availableUpdate) {
      this.updatePromptOpen = true;
      return;
    }

    this.updateStatus = 'checking';
    this.updateError = '';

    try {
      const update = await this.appUpdates.checkForUpdate();
      if (!update) {
        this.updateStatus = manual ? 'up-to-date' : 'idle';
        return;
      }

      this.availableUpdate = update;
      this.updateStatus = 'available';
      this.updatePromptOpen = true;
    } catch (error) {
      this.updateError = this.errorMessage(error);
      this.updateStatus = manual ? 'error' : 'idle';
    }
  }

  postponeUpdate() {
    if (this.updateStatus === 'downloading' || this.updateStatus === 'installing') {
      return;
    }
    this.updatePromptOpen = false;
  }

  async installUpdate() {
    if (!this.availableUpdate || this.updateStatus === 'downloading' || this.updateStatus === 'installing') {
      return;
    }

    this.updateStatus = 'downloading';
    this.updateDownloadedBytes = 0;
    this.updateTotalBytes = 0;
    this.updateError = '';

    try {
      await this.appUpdates.downloadAndInstall((event) => {
        this.zone.run(() => {
          if (event.event === 'Started') {
            this.updateTotalBytes = event.data.contentLength ?? 0;
          } else if (event.event === 'Progress') {
            this.updateDownloadedBytes += event.data.chunkLength;
          } else {
            this.updateStatus = 'installing';
          }
        });
      });
    } catch (error) {
      this.updateError = this.errorMessage(error);
      this.updateStatus = 'available';
    }
  }

  get updateSettingsText() {
    switch (this.updateStatus) {
      case 'checking':
        return 'Checking for a new version…';
      case 'available':
        return `Version ${this.availableUpdate?.version} is available`;
      case 'downloading':
        return this.updateTotalBytes > 0
          ? `Downloading ${Math.round(this.updateDownloadProgress)}%…`
          : 'Downloading update…';
      case 'installing':
        return 'Installing update…';
      case 'up-to-date':
        return `FileDrop ${this.appVersion} is up to date`;
      case 'error':
        return 'Could not check. Please try again.';
      default:
        return 'Get the newest features and fixes';
    }
  }

  get updateDownloadProgress() {
    if (this.updateTotalBytes <= 0) {
      return 0;
    }
    return Math.min(100, (this.updateDownloadedBytes / this.updateTotalBytes) * 100);
  }

  get desktopUpdatesSupported() {
    return this.localPlatform === 'mac' || this.localPlatform === 'windows';
  }

  async toggleDiscoverability() {
    try {
      const settings = await invoke<AppSettings>('set_discoverable', {
        enabled: !this.discoverable,
      });
      this.applySettings(settings);
    } catch (error) {
      this.transferError = this.errorMessage(error);
    }
  }

  async toggleAutoOpenReceived() {
    try {
      const settings = await invoke<AppSettings>('set_auto_open_received', {
        enabled: !this.autoOpenReceived,
      });
      this.applySettings(settings);
    } catch (error) {
      this.transferError = this.errorMessage(error);
    }
  }

  openFileSelection(device: NearbyDevice) {
    this.resetTransferState();
    this.selectedDevice = device;
    this.selectedFiles = [];
    this.fileSelectionOpen = true;
  }

  closeFileSelection() {
    this.fileSelectionOpen = false;
    this.selectedDevice = null;
    this.selectedFiles = [];
  }

  async pickFiles() {
    if (!this.runningInTauri) {
      return;
    }

    const selected = await open({
      multiple: true,
      directory: false,
      title: 'Select files to send',
    });

    if (Array.isArray(selected)) {
      await this.addFilePaths(selected);
    } else if (selected) {
      await this.addFilePaths([selected]);
    }
  }

  removeFile(fileIndex: number) {
    this.selectedFiles = this.selectedFiles.filter((_, index) => index !== fileIndex);
  }

  async startSending() {
    if (!this.selectedDevice || this.selectedFiles.length === 0) {
      return;
    }

    this.fileSelectionOpen = false;
    this.sendingOpen = true;
    this.transferError = '';
    this.transferTotalBytes = this.selectedTotalBytes;
    this.transferRemainingBytes = this.transferTotalBytes;
    const requestGeneration = ++this.sendRequestGeneration;

    try {
      const transferId = await invoke<string>('send_files', {
        deviceId: this.selectedDevice.id,
        paths: this.selectedFiles.map((file) => file.path),
      });
      if (requestGeneration !== this.sendRequestGeneration || !this.sendingOpen) {
        await invoke('cancel_transfer', { transferId });
        return;
      }
      this.activeTransferId = transferId;
    } catch (error) {
      if (requestGeneration === this.sendRequestGeneration) {
        this.transferError = this.errorMessage(error);
      }
    }
  }

  async cancelSending() {
    this.sendRequestGeneration += 1;
    await this.cancelActiveTransfer();
    this.sendingOpen = false;
    this.selectedDevice = null;
    this.selectedFiles = [];
    this.resetTransferState();
  }

  async respondToIncomingOffer(accepted: boolean) {
    if (!this.incomingOffer) {
      return;
    }

    const offer = this.incomingOffer;
    this.transferError = '';

    try {
      await invoke('respond_to_offer', {
        transferId: offer.transferId,
        accepted,
      });
    } catch (error) {
      this.transferError = this.errorMessage(error);
      return;
    }

    this.receivingConfirmationOpen = false;
    if (accepted) {
      this.activeTransferId = offer.transferId;
      this.receivingOpen = true;
      this.transferTotalBytes = offer.totalBytes;
      this.transferRemainingBytes = offer.totalBytes;
    } else {
      this.incomingOffer = null;
      this.resetTransferState();
    }
  }

  async cancelReceiving() {
    await this.cancelActiveTransfer();
    this.receivingOpen = false;
    this.incomingOffer = null;
    this.resetTransferState();
  }

  async closeActiveTransfer() {
    if (this.sendingOpen) {
      await this.cancelSending();
    } else {
      await this.cancelReceiving();
    }
  }

  transferFiles(): Array<{ name: string }> {
    if (this.sendingOpen) {
      return this.selectedFiles;
    }
    return this.incomingOffer?.files ?? [];
  }

  isFileComplete(fileIndex: number) {
    return this.transferFinished || fileIndex < this.completedFiles;
  }

  isFileActive(fileIndex: number) {
    return !this.transferFinished && fileIndex === this.currentFileIndex;
  }

  get selectedTotalBytes() {
    return this.selectedFiles.reduce((total, file) => total + file.size, 0);
  }

  get homeStatusText() {
    if (!this.networkOnline) {
      return 'Offline — connect to a network';
    }
    if (!this.discoverable) {
      return 'Not discoverable — sharing is disabled';
    }
    return `Discoverable as ${this.localDeviceName}`;
  }

  get fileManagerName() {
    return this.localPlatform === 'windows' ? 'File Explorer' : 'Finder';
  }

  formatBytes(bytes: number) {
    if (!Number.isFinite(bytes) || bytes <= 0) {
      return '0 MB';
    }
    const units = ['B', 'KB', 'MB', 'GB', 'TB'];
    const unitIndex = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
    const value = bytes / 1024 ** unitIndex;
    const decimals = unitIndex < 2 || value >= 100 ? 0 : value >= 10 ? 1 : 2;
    return `${value.toFixed(decimals)} ${units[unitIndex]}`;
  }

  formatSpeed(bytesPerSecond: number) {
    return bytesPerSecond > 0 ? `${this.formatBytes(bytesPerSecond)}/s` : 'Calculating speed…';
  }

  async revealReceivedFiles() {
    const firstSavedFile = this.receivedSavedFiles[0];
    if (!firstSavedFile) {
      return;
    }
    try {
      await revealItemInDir(firstSavedFile);
    } catch (error) {
      this.transferError = `Could not open ${this.fileManagerName}: ${this.errorMessage(error)}`;
    }
  }

  private async registerBackendListeners() {
    const devicesUnlisten = await listen<NearbyDevice[]>('devices-changed', (event) => {
      this.zone.run(() => {
        this.devices = event.payload;
      });
    });

    const settingsUnlisten = await listen<AppSettings>('settings-changed', (event) => {
      this.zone.run(() => this.applySettings(event.payload));
    });

    const networkUnlisten = await listen<boolean>('network-status-changed', (event) => {
      this.zone.run(() => {
        this.networkOnline = event.payload;
      });
    });

    const offerUnlisten = await listen<TransferOffer>('incoming-offer', (event) => {
      this.zone.run(() => this.handleIncomingOffer(event.payload));
    });

    const progressUnlisten = await listen<TransferProgress>('transfer-progress', (event) => {
      this.zone.run(() => this.handleTransferProgress(event.payload));
    });

    const finishedUnlisten = await listen<TransferFinished>('transfer-finished', (event) => {
      this.zone.run(() => this.handleTransferFinished(event.payload));
    });

    const failedUnlisten = await listen<TransferFailed>('transfer-failed', (event) => {
      this.zone.run(() => this.handleTransferFailed(event.payload));
    });

    this.unlistenFunctions.push(
      devicesUnlisten,
      settingsUnlisten,
      networkUnlisten,
      offerUnlisten,
      progressUnlisten,
      finishedUnlisten,
      failedUnlisten,
    );
  }

  private handleIncomingOffer(offer: TransferOffer) {
    if (this.incomingOffer || this.sendingOpen || this.receivingOpen) {
      void invoke('respond_to_offer', {
        transferId: offer.transferId,
        accepted: false,
      });
      return;
    }

    this.resetTransferState();
    this.incomingOffer = offer;
    if (this.settingsCloseTimer) {
      clearTimeout(this.settingsCloseTimer);
      this.settingsCloseTimer = null;
    }
    this.settingsOpen = false;
    this.settingsClosing = false;
    this.fileSelectionOpen = false;
    this.selectedDevice = null;
    this.selectedFiles = [];
    this.receivingConfirmationOpen = true;
  }

  private handleTransferProgress(progress: TransferProgress) {
    if (this.activeTransferId && progress.transferId !== this.activeTransferId) {
      return;
    }

    this.activeTransferId = progress.transferId;
    this.transferProgress = progress.progress;
    this.completedFiles = progress.completedFiles;
    this.currentFileIndex = progress.currentFileIndex;
    this.transferTransferredBytes = progress.transferredBytes;
    this.transferTotalBytes = progress.totalBytes;
    this.transferRemainingBytes = progress.remainingBytes;
    this.transferBytesPerSecond = progress.bytesPerSecond;
  }

  private handleTransferFinished(finished: TransferFinished) {
    if (finished.transferId !== this.activeTransferId) {
      return;
    }

    this.transferProgress = 100;
    this.completedFiles = this.transferFiles().length;
    this.transferFinished = true;
    this.transferTransferredBytes = this.transferTotalBytes;
    this.transferRemainingBytes = 0;
    this.transferBytesPerSecond = 0;
    this.receivedSavedFiles = finished.savedFiles;
    if (finished.direction === 'receiving' && this.autoOpenReceived) {
      void this.revealReceivedFiles();
    }
  }

  private handleTransferFailed(failed: TransferFailed) {
    if (this.activeTransferId && failed.transferId !== this.activeTransferId) {
      return;
    }
    this.transferError = failed.message;
  }

  private async addFilePaths(paths: string[]) {
    const knownPaths = new Set(this.selectedFiles.map((file) => file.path));
    const newPaths = paths.filter((path) => !knownPaths.has(path));
    if (newPaths.length === 0) {
      return;
    }
    try {
      const additions = await invoke<SelectedFile[]>('inspect_files', { paths: newPaths });
      this.selectedFiles = [...this.selectedFiles, ...additions];
    } catch (error) {
      this.transferError = this.errorMessage(error);
    }
  }

  private applySettings(settings: AppSettings) {
    this.autoOpenReceived = settings.autoOpenReceived;
    this.discoverable = settings.discoverable;
  }

  private async cancelActiveTransfer() {
    if (!this.activeTransferId || this.transferFinished) {
      return;
    }

    try {
      await invoke('cancel_transfer', { transferId: this.activeTransferId });
    } catch {
      // The backend may already have finished and removed the cancellation token.
    }
  }

  private resetTransferState() {
    this.activeTransferId = null;
    this.transferProgress = 0;
    this.completedFiles = 0;
    this.currentFileIndex = 0;
    this.transferFinished = false;
    this.transferError = '';
    this.transferTransferredBytes = 0;
    this.transferTotalBytes = 0;
    this.transferRemainingBytes = 0;
    this.transferBytesPerSecond = 0;
    this.receivedSavedFiles = [];
  }

  private errorMessage(error: unknown) {
    return error instanceof Error ? error.message : String(error);
  }
}
