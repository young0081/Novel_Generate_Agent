# Desktop Application

The desktop client is a Tauri 2 application. It embeds the Rust core directly;
it does not require the Next.js server or a separate `na-host` process.

## Requirements

- Rust 1.80 or newer
- Node.js 20.19 or newer
- Windows WebView2 and the MSVC build tools

## Development

```bash
npm ci
npm run build
npm run tauri dev
```

Run the native checks from `src-tauri`:

```bash
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
cargo fmt -- --check
```

## Release

```bash
npm run tauri build -- --bundles nsis
```

The application identifier is `com.novelgenerateagent.desktop`. User data is
stored in the corresponding Tauri application-data directory. On first launch,
the desktop app imports app-owned data from the pre-rename
`com.novelgenerateteam.desktop` directory without deleting that legacy copy.

## Bulk management

Lists that can grow over time expose a `批量管理` action: memory, sessions,
works, knowledge bases and entries, checkpoints, revision files, IDE workspace
files, and model providers. Select visible items, confirm once, and the client
deletes them sequentially with progress and partial-failure feedback. The
active work and protected directories are never included in bulk deletion.
