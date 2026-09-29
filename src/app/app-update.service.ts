import { Injectable } from '@angular/core';
import {
  check,
  DownloadEvent,
  Update,
} from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';

export interface AvailableAppUpdate {
  currentVersion: string;
  version: string;
  date?: string;
  notes?: string;
}

@Injectable({ providedIn: 'root' })
export class AppUpdateService {
  private pendingUpdate: Update | null = null;
  private checkInFlight: Promise<AvailableAppUpdate | null> | null = null;

  async checkForUpdate(): Promise<AvailableAppUpdate | null> {
    if (this.pendingUpdate) {
      return this.toAvailableUpdate(this.pendingUpdate);
    }

    if (this.checkInFlight) {
      return this.checkInFlight;
    }

    this.checkInFlight = this.performCheck();
    try {
      return await this.checkInFlight;
    } finally {
      this.checkInFlight = null;
    }
  }

  async downloadAndInstall(onEvent: (event: DownloadEvent) => void): Promise<void> {
    if (!this.pendingUpdate) {
      throw new Error('The update is no longer available. Check for updates again.');
    }

    await this.pendingUpdate.downloadAndInstall(onEvent, {
      restartAfterInstall: true,
    });

    // Windows normally exits during installation. macOS returns here and needs
    // an explicit relaunch to start the newly installed version.
    await relaunch();
  }

  async dispose(): Promise<void> {
    if (!this.pendingUpdate) {
      return;
    }

    const update = this.pendingUpdate;
    this.pendingUpdate = null;
    await update.close();
  }

  private async performCheck(): Promise<AvailableAppUpdate | null> {
    const update = await check({ timeout: 20_000 });
    if (!update) {
      return null;
    }

    this.pendingUpdate = update;
    return this.toAvailableUpdate(update);
  }

  private toAvailableUpdate(update: Update): AvailableAppUpdate {
    return {
      currentVersion: update.currentVersion,
      version: update.version,
      date: update.date,
      notes: update.body,
    };
  }
}
