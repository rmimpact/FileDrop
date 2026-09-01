# FileDrop installers

FileDrop produces two normal desktop installers:

- **macOS:** a universal `.dmg` for both Apple Silicon and Intel Macs. Open it and drag FileDrop into Applications.
- **Windows:** a 64-bit NSIS `-setup.exe`. It installs for the current user without requiring administrator access. It checks for Microsoft Edge WebView2 and downloads and installs it only when it is missing.

Use these permanent direct-download links on websites and in app catalogues:

- **macOS:** `https://github.com/rmimpact/FileDrop/releases/latest/download/FileDrop-macOS.dmg`
- **Windows:** `https://github.com/rmimpact/FileDrop/releases/latest/download/FileDrop-Windows-x64-Setup.exe`

GitHub redirects each link to the matching installer in the newest published release. The workflow keeps the asset names stable, so these URLs never need a version-number update.

The recipient does not need Node.js, Rust, Visual Studio, VS Code, Codex, or the repository. Those are development tools used only while building FileDrop.

## Build both installers

1. Open the repository on GitHub.
2. Open **Actions**.
3. Choose **Build FileDrop installers**.
4. Select **Run workflow**.
5. When both jobs finish, open the draft release named for the current FileDrop version.
6. Download and test the DMG and setup EXE, then publish the draft release when ready.

GitHub builds the DMG on a macOS runner and the EXE on a Windows runner, so both installers can be produced while working only from a Mac.

## Publish a new version

FileDrop releases follow normal version numbers such as `1.0.0`, `1.1.0`, and `2.0.0`.

1. Update the version in `package.json`, `package-lock.json`, `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`, and the frontend fallback in `src/app/app.component.ts`.
2. Commit and push the version change.
3. Create and push a matching tag such as `filedrop-v1.1.0`.
4. GitHub automatically builds both installers and publishes a release with generated release notes.

For a test build that should remain private, run the workflow manually from the Actions page instead. Manual runs create draft releases.

Existing users update by downloading the newer installer and installing it over their current FileDrop installation. Their persistent device identity and settings remain in the operating system's application-data directory. In-app automatic update checks are not enabled yet; those require a separately secured updater-signing key.

## Build the Mac installer locally

Run `npm run installer:mac`. The DMG is created under `src-tauri/target/release/bundle/dmg`.

The Windows command is `npm run installer:windows`, but it must be run on Windows. The GitHub workflow is the recommended way to build it.

## One-time Mac signing setup

The release workflow signs the Mac app with an Apple **Developer ID Application** certificate, submits the DMG to Apple's automated notary service, and staples the approval ticket to the finished installer. This is the correct distribution method for FileDrop because it is downloaded directly rather than installed through the Mac App Store.

Do not put certificates, passwords, private keys, or their encoded contents in this repository. Store all six values below as GitHub Actions repository secrets under **Settings → Secrets and variables → Actions**.

### 1. Create and install the certificate

Only the Apple Developer Account Holder can create a Developer ID certificate.

1. Open **Keychain Access** on the Mac.
2. Choose **Keychain Access → Certificate Assistant → Request a Certificate from a Certificate Authority**.
3. Enter the Apple Developer account email and a descriptive common name, leave the CA email blank, select **Saved to disk**, and save the certificate request.
4. Sign in to [Apple Developer Certificates](https://developer.apple.com/account/resources/certificates/list), create a certificate, choose **Developer ID Application**, and upload the certificate request.
5. Download the `.cer` file and double-click it to install it in the login keychain.
6. In Keychain Access, open **login → My Certificates**. The new Developer ID Application certificate must have a private key nested underneath it.

A **Developer ID Installer** certificate is not required for the FileDrop DMG. That certificate type is for `.pkg` installers.

### 2. Export the certificate for GitHub

1. In **Keychain Access → login → My Certificates**, expand the Developer ID Application certificate.
2. Select the certificate and its private key, then export them together as a password-protected `.p12` file.
3. Choose a strong one-time export password. Add it to GitHub as the `APPLE_CERTIFICATE_PASSWORD` secret.
4. Convert the `.p12` file to a single-line Base64 value:

   ```sh
   openssl base64 -A -in /path/to/filedrop-developer-id.p12
   ```

5. Copy the command's output into the `APPLE_CERTIFICATE` GitHub secret.
6. Generate another long random password and save it as the `KEYCHAIN_PASSWORD` GitHub secret. It protects only the temporary Keychain created on GitHub's build runner.
7. Securely archive the `.p12` file and its password as a signing-key backup, then delete unneeded working copies.

### 3. Add Apple notarization credentials

1. Go to [Apple Account Sign-In and Security](https://account.apple.com/account/manage), create an **app-specific password**, and label it `FileDrop GitHub notarization`.
2. Add the app-specific password to GitHub as `APPLE_PASSWORD`. Do not use the normal Apple account password.
3. Add the Apple account email as `APPLE_ID`.
4. Find the 10-character Team ID on the [Apple Developer Membership](https://developer.apple.com/account#MembershipDetailsCard) page and add it as `APPLE_TEAM_ID`.

The six required repository secrets are therefore:

| GitHub secret | Value |
| --- | --- |
| `APPLE_CERTIFICATE` | Single-line Base64 contents of the exported `.p12` certificate |
| `APPLE_CERTIFICATE_PASSWORD` | Password chosen when exporting the `.p12` certificate |
| `KEYCHAIN_PASSWORD` | A new random password for GitHub's temporary build Keychain |
| `APPLE_ID` | Apple Developer account email address |
| `APPLE_PASSWORD` | Apple app-specific password, not the normal account password |
| `APPLE_TEAM_ID` | Apple Developer Team ID |

### 4. Produce and verify the signed installer

Run **Build FileDrop installers** manually from GitHub Actions. The result remains a draft release so it can be tested before publication.

After downloading the DMG, verify it on a Mac:

```sh
codesign --verify --deep --strict --verbose=2 "/Applications/FileDrop.app"
spctl --assess --type execute --verbose=4 "/Applications/FileDrop.app"
xcrun stapler validate "/path/to/FileDrop.dmg"
```

The first public test should also be performed on a different Mac that has never run FileDrop. Download the DMG through a browser so macOS applies its normal internet quarantine checks.

Signing changes installer trust only; it does not change FileDrop's transfer features. Windows still needs a separate Windows code-signing certificate to avoid an unknown-publisher or SmartScreen warning.
