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
}