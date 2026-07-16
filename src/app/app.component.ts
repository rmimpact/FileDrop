import { Component } from '@angular/core';

type DeviceType = 'mac' | 'windows';

interface NearbyDevice {
  id: number;
  name: string;
  type: DeviceType;
}

@Component({
  selector: 'app-root',
  standalone: true,
  templateUrl: './app.component.html',
  styleUrl: './app.component.css',
})
export class AppComponent {

  // ===== Current device =====
  localDeviceName = "Jayden’s Macbook";

  // ===== Settings =====
  settingsOpen = false;

  selectedTheme: 'auto' | 'light' | 'dark' = 'auto';

  // ===== File selection =====
  fileSelectionOpen = false;

  selectedDevice: NearbyDevice | null = null;

  selectedFiles: string[] = ['Code.c', 'Image.png'];

  // ===== Nearby devices =====
  devices: NearbyDevice[] = [
    {
      id: 1,
      name: "Remy’s Macbook",
      type: 'mac',
    },
    {
      id: 2,
      name: "Remy’s PC",
      type: 'windows',
    },
    {
      id: 3,
      name: "Testing Long Name PC",
      type: 'windows',
    },
    {
      id: 4,
      name: "Testing Long Name PC",
      type: 'mac',
    },
  ];

  // ===== Functions =====

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
    this.selectedDevice = device;
    this.fileSelectionOpen = true;
  }

  closeFileSelection() {
    this.fileSelectionOpen = false;
    this.selectedDevice = null;
  }

  addFiles(fileList: FileList | null) {
    if (!fileList) {
      return;
    }

    const incomingFiles = Array.from(fileList).map((file) => file.name);
    this.selectedFiles = [...this.selectedFiles, ...incomingFiles];
  }

  removeFile(fileIndex: number) {
    this.selectedFiles = this.selectedFiles.filter((_, index) => index !== fileIndex);
  }

  onDrop(event: DragEvent) {
    event.preventDefault();
    this.addFiles(event.dataTransfer?.files ?? null);
  }

  onDragOver(event: DragEvent) {
    event.preventDefault();
  }
}
