# Releasing Soniq Studio

Releases are built by GitHub Actions (`.github/workflows/release.yml`) when a
version tag is pushed. Installed copies find them through the updater.

## A new version

1. `node scripts/set-version.mjs 0.2.0` sets the version in `package.json`,
   `Cargo.toml` and `src-tauri/tauri.conf.json`.
2. Commit, tag and push: `git tag v0.2.0 && git push origin main v0.2.0`.
3. The Release workflow builds:
   - **Windows**: the NSIS setup (`Soniq Studio_0.2.0_x64-setup.exe`), a per-user
     install with no admin prompt, Start menu entry and uninstaller;
   - **macOS** (Apple silicon): `.app` and `.dmg`, unsigned and not notarised;
   - **Linux**: AppImage and `.deb`.

   macOS and Linux are best effort: if they fail, the release still goes out
   with Windows. When Windows has built, the release is published as the latest.
4. Each build uploads its update file with a signature, and `latest.json`,
   which installed apps read from
   `https://github.com/Voltiq/soniq-studio/releases/latest/download/latest.json`.

## The update key

Updates are signed with a private key; the app only installs updates signed
with it (the public half is `plugins.updater.pubkey` in `tauri.conf.json`).

- The private key is at `%USERPROFILE%\.tauri\soniq-studio.key` on the machine
  that made it, and in the repository secret `TAURI_SIGNING_PRIVATE_KEY`. It
  has no password.
- **Keep a backup of it somewhere safe.** If it is lost, installed copies can no
  longer update: you would need a new key, and everyone would have to install
  the next version by hand.
- Building locally with update files needs it in the environment:
  `TAURI_SIGNING_PRIVATE_KEY` (the key text or its path) and an empty
  `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.

## Code signing (not set up yet)

The Windows setup is not code-signed, so SmartScreen warns on first run
("More info › Run anyway"). To sign it:

- **A certificate** (from a certificate authority, installed in the Windows
  certificate store): set `bundle.windows.certificateThumbprint`, plus
  `digestAlgorithm: "sha256"` and `timestampUrl`, in `tauri.conf.json`. On CI
  the certificate has to be imported first (as a base64 `.pfx` secret).
- **Azure Trusted Signing**: set `bundle.windows.signCommand` to
  `trusted-signing-cli -e <endpoint> -a <account> -c <profile> %1`, with the
  Azure credentials as secrets.

macOS needs an Apple Developer ID to sign and notarise (`APPLE_CERTIFICATE`,
`APPLE_ID`, `APPLE_TEAM_ID`… secrets, which tauri-action reads).

## Crash reports

Crash reports are opt-in and stay on the user's computer
(`%APPDATA%\app.soniq.studio\crash-reports`). Users can copy one into a GitHub
issue from Help › Crash reports.
