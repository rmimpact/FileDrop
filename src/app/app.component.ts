import { Component, NgZone, OnDestroy, OnInit } from '@angular/core';
import { NgTemplateOutlet } from '@angular/common';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { open } from '@tauri-apps/plugin-dialog';

type DeviceType = 'mac' | 'windows' | 'other';
type TransferDirection = 'sending' | 'receiving';

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
}

interface SelectedFile {
  name: string;
  path: string;
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
  downloadDirectory = '';
  settingsOpen = false;
  selectedTheme: 'auto' | 'light' | 'dark' = 'auto';

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

  private unlistenFunctions: UnlistenFn[] = [];
  private readonly runningInTauri = '__TAURI_INTERNALS__' in window;
  private sendRequestGeneration = 0;

  constructor(private readonly zone: NgZone) {}

  async ngOnInit() {
    if (!this.runningInTauri) {
      return;
    }

    await this.registerBackendListeners();

    try {
      const [appInfo, devices] = await Promise.all([
        invoke<AppInfo>('get_app_info'),
        invoke<NearbyDevice[]>('get_nearby_devices'),
      ]);
      this.localDeviceName = appInfo.identity.name;
      this.downloadDirectory = appInfo.downloadDirectory;
      this.devices = devices;

      const unlistenDragDrop = await getCurrentWebview().onDragDropEvent((event) => {
        if (event.payload.type === 'drop' && this.fileSelectionOpen) {
          const paths = event.payload.paths;
          this.zone.run(() => this.addFilePaths(paths));
        }
      });
      this.unlistenFunctions.push(unlistenDragDrop);
    } catch (error) {
      this.transferError = this.errorMessage(error);
    }
  }

  ngOnDestroy() {
    for (const unlisten of this.unlistenFunctions) {
      unlisten();
    }
  }

  openSettings() {
    this.settingsOpen = true;
  }

  closeSettings() {
    this.settingsOpen = false;
  }

  setTheme(theme: 'auto' | 'light' | 'dark') {
    this.selectedTheme = theme;
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
      this.addFilePaths(selected);
    } else if (selected) {
      this.addFilePaths([selected]);
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

  private async registerBackendListeners() {
    const devicesUnlisten = await listen<NearbyDevice[]>('devices-changed', (event) => {
      this.zone.run(() => {
        this.devices = event.payload;
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
    this.settingsOpen = false;
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
  }

  private handleTransferFinished(finished: TransferFinished) {
    if (finished.transferId !== this.activeTransferId) {
      return;
    }

    this.transferProgress = 100;
    this.completedFiles = this.transferFiles().length;
    this.transferFinished = true;
  }

  private handleTransferFailed(failed: TransferFailed) {
    if (this.activeTransferId && failed.transferId !== this.activeTransferId) {
      return;
    }
    this.transferError = failed.message;
  }

  private addFilePaths(paths: string[]) {
    const knownPaths = new Set(this.selectedFiles.map((file) => file.path));
    const additions = paths
      .filter((path) => !knownPaths.has(path))
      .map((path) => ({
        path,
        name: path.split(/[\\/]/).pop() || path,
      }));
    this.selectedFiles = [...this.selectedFiles, ...additions];
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
  }

  private errorMessage(error: unknown) {
    return error instanceof Error ? error.message : String(error);
  }
}
