# Processing model

## Trust boundary

The React interface submits structured operations to Rust. Rust canonicalises every input and the chosen output directory, recognises only an allowlisted set of extensions, and turns the operation model into explicit process arguments. No operation accepts arbitrary command text and no shell is involved.

Each output is first written as a hidden, job-specific temporary file in the destination directory. Forge renames it to the final collision-safe name only after the processor exits successfully. Cancellation kills the active child, removes its temporary file, and leaves completed files plus every original intact.

## Images

ImageMagick handles JPEG, PNG, WebP, AVIF, supported HEIC/HEIF inputs, GIF, and TIFF. Input orientation is normalised before recipe operations. Resize supports fit-within, crop-to-fill, and exact dimensions. Quality maps directly to ImageMagick's quality setting. Metadata removal uses `-strip`, then inspects the result for remaining EXIF properties before accepting it.

Animated GIF input is accepted but this release does not promise animation-preserving output. Colour-profile removal is part of stripping metadata, so colour-managed professional workflows should keep an unstripped preset.

## Video

FFmpeg/ffprobe handle MP4, MOV, MKV, WebM, AVI, and M4V input. Exports use H.264/AAC in MP4 or VP9/Opus in WebM. Quality is mapped to a bounded constant-rate-factor value. Fit resizing preserves aspect ratio; fill resizing crops centrally. Rotation, trim ranges, and audio removal are supported.

Actual codec availability depends on the system FFmpeg build. Forge reports per-file processor failures and preserves successful siblings in the batch.

Video previews are real frames rendered from the active trim, resize, and rotation recipe. FFmpeg's structured progress stream drives within-file progress during encoding. Final video dimensions are rounded down by at most one pixel when necessary for codecs using 4:2:0 pixel formats.

## Recipe portability and validation

Recipes can contain at most 20 enabled operations. Dimensions, quality, trim ranges, rotations, output formats, and operation names are validated independently in the interface and native backend. Only one format decision is allowed. Imported preset files are limited to 1 MiB, parsed as structured JSON, and pass the same native validation before entering local state. Presets contain configuration only and cannot carry commands or executable paths.

## Estimates and history

Output sizes shown before processing are estimates based on pixel-area and quality heuristics, and are labelled accordingly. The visual “after” pane is separately rendered from the active recipe; it is not the original image with a label. Completed size values come from the exported file. History stores counts, byte totals, timestamps, destinations, and error summaries—not file contents.

## Limits

- A batch is limited to 1,000 direct files.
- Folder import is intentionally non-recursive.
- Originals are never overwritten.
- Existing exports receive `-2`, `-3`, and later suffixes.
- Free-space checking conservatively requires roughly the batch's original size plus ten percent.
- Interrupted applications may leave a hidden `.forge-*` file only if the entire process is killed before Forge can run cleanup; it is safe to delete.
