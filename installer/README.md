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

When rebuilding an already archived version, first copy its complete release
directory to a separate dated backup and verify the copied SHA-256 hashes.
After validation, synchronize `dist-out/NovelGenerateAgent-Setup-vX.Y.Z.exe`
with the custom installer in `releases/vX.Y.Z/`; both files must have the same
hash. Keep release notes alongside the checksums to identify the final build.

For signed builds, verify the desktop executable again after NSIS bundling:
the bundler may restore the original executable after signing its bundled
copy. Sign the final desktop payload before copying it into this installer,
then sign and verify the custom installer. Verify that its embedded payload
is byte-for-byte identical to the archived desktop executable.

## Verification

```bash
npm run build
cargo test --manifest-path src-tauri/Cargo.toml --lib
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```
