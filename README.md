# Forge

Forge is a private, local-first desktop toolbox for routine image and video work. Drop in files, arrange a visible processing recipe, compare the estimated result, and export a collision-safe batch without uploading anything.

The first release supports:

- image resizing, JPEG/PNG/WebP/AVIF conversion, quality control, rotation, and metadata removal;
- video trimming, resizing, MP4/WebM conversion, compression, rotation, and audio removal;
- drag-and-drop, native file and output-folder pickers, folder import, batches up to 1,000 files, and per-file results;
- reusable built-in and custom presets with validated JSON import and export;
- genuinely rendered before/after image and video-frame previews, plus conservative size estimates;
- live per-video encoding progress from FFmpeg;
- cancellation, temporary-output cleanup, free-space checking, collision-safe names, and local job history; and
- a browser-safe demo adapter for interface development.

Originals are never modified. Processing is performed through explicit ImageMagick and FFmpeg argument arrays; Forge never constructs shell commands.

## Requirements

- Node.js 22 or newer
- Rust stable
- Tauri 2 Linux development dependencies
- ImageMagick 7 (`magick`)
- FFmpeg and ffprobe
- KDE `kdialog` or GTK `zenity` for native pickers

On Fedora, the non-language dependencies can be installed with:

```bash
sudo dnf install ffmpeg ImageMagick kdialog zenity webkit2gtk4.1-devel openssl-devel libappindicator-gtk3-devel librsvg2-devel
```

## Develop

```bash
npm install
npm run desktop
```

Use `npm run dev` for the browser demo. It deliberately cannot access or process local files.

## Verify

```bash
npm test
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml
```

## Package for Fedora

```bash
npm run desktop:build
```

The RPM and AppImage are written below `src-tauri/target/release/bundle/`.

## Processing and safety details

See [docs/PROCESSING.md](docs/PROCESSING.md) for operation behavior, supported formats, metadata verification, cancellation, and known format limitations. Release checks are in [docs/RELEASE_CHECKLIST.md](docs/RELEASE_CHECKLIST.md).

## Local data

Presets, the preferred export folder, and up to 100 job summaries are stored in the platform local-data directory under `forge/state.json`. Source media and exported contents are not copied into the application data directory. Remove that file to reset Forge.

## Licensing

Forge itself can be licensed independently. ImageMagick and the system FFmpeg build are external runtime tools. Before distributing bundled executables, audit the exact FFmpeg build configuration and codec licences; enabling GPL or non-free components changes distribution obligations. This development release uses system-installed binaries and does not redistribute either processor.
