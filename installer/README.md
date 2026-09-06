# Windows Installer

This Tauri 2 application produces the per-user, single-file Windows installer
for Novel Generate Agent. It installs under `%LOCALAPPDATA%` by default and does
not require elevation.

## Requirements

- Node.js 20.19 or newer
- Rust 1.80 or newer with the MSVC toolchain
- A release desktop payload at
  `src-tauri/payload/NovelGenerateAgent.exe`

## Build

Build the desktop application first, then copy its executable into the payload
directory:

```bash
cd ../desktop-tauri
npm ci
npm run tauri build -- --bundles nsis
cp src-tauri/target/release/desktop-tauri.exe ../installer/src-tauri/payload/NovelGenerateAgent.exe

cd ../installer
npm ci
npm run build
npm run tauri build -- --no-bundle
```

The release artifact is `src-tauri/target/release/installer.exe`. Release builds
fail when the real payload is missing; debug checks use a non-installable stub
so unit tests and CI can run from a clean checkout.

Completed installers are archived at the repository root in `releases/vX.Y.Z/`.
For each release, keep the Tauri NSIS package and the custom installer together
with a `SHA256SUMS.txt` file. Include the desktop payload copy when it is
available for troubleshooting or reproducibility.

## Verification

```bash
npm run build
cargo test --manifest-path src-tauri/Cargo.toml --lib
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```
