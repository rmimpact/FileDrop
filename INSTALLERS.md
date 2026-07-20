# FileDrop installers

FileDrop produces two normal desktop installers:

- **macOS:** a universal `.dmg` for both Apple Silicon and Intel Macs. Open it and drag FileDrop into Applications.
- **Windows:** a 64-bit NSIS `-setup.exe`. It installs for the current user without requiring administrator access. It checks for Microsoft Edge WebView2 and downloads and installs it only when it is missing.

The recipient does not need Node.js, Rust, Visual Studio, VS Code, Codex, or the repository. Those are development tools used only while building FileDrop.

## Build both installers

1. Open the repository on GitHub.
2. Open **Actions**.
3. Choose **Build FileDrop installers**.
4. Select **Run workflow**.
5. When both jobs finish, open the draft release named for the current FileDrop version.
6. Download and test the DMG and setup EXE, then publish the draft release when ready.

GitHub builds the DMG on a macOS runner and the EXE on a Windows runner, so both installers can be produced while working only from a Mac.

## Build the Mac installer locally

Run `npm run installer:mac`. The DMG is created under `src-tauri/target/release/bundle/dmg`.

The Windows command is `npm run installer:windows`, but it must be run on Windows. The GitHub workflow is the recommended way to build it.

## Signing before public distribution

The current workflow creates test-ready installers without paid signing certificates. For a warning-free public release:

- macOS requires an Apple Developer ID certificate and Apple notarization.
- Windows requires a code-signing certificate to avoid an unknown-publisher or SmartScreen warning.

Signing changes installer trust only; it does not change FileDrop's transfer features.
