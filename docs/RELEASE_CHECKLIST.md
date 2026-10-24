# Release checklist

- Run `npm test`, `npm run build`, `cargo test`, and `cargo check`.
- Process representative JPEG, PNG transparency, WebP, AVIF, HEIC, MP4, MOV, and WebM samples.
- Verify EXIF and GPS removal on exported photos.
- Cancel one large image and one long video; confirm incomplete temporary files are removed.
- Export duplicate names and confirm no existing file is replaced.
- Test insufficient disk space and an unwritable destination.
- Inspect the RPM and AppImage on a clean supported Fedora installation.
- Confirm the package invokes system ImageMagick and FFmpeg, or complete a codec/licence audit before bundling them.
- Check keyboard navigation, focus visibility, high scaling, long filenames, and error readability.
- Record the release's ImageMagick and FFmpeg versions and supported codec matrix.
