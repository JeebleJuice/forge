import type { Preset } from "./types";

const op = (id: string, kind: Preset["operations"][number]["kind"], extra = {}) => ({ id, kind, enabled: true, ...extra });

export const BUILTIN_PRESETS: Preset[] = [
  { id: "email", name: "Email attachment", description: "Compact JPEG, longest edge 1600 px", builtin: true, operations: [op("resize-email", "resize", { width: 1600, height: 1600, fit: "contain" }), op("format-email", "format", { format: "jpg" }), op("quality-email", "quality", { quality: 78 }), op("strip-email", "stripMetadata")] },
  { id: "web", name: "Website image", description: "WebP at 82%, up to 2048 px", builtin: true, operations: [op("resize-web", "resize", { width: 2048, height: 2048, fit: "contain" }), op("format-web", "format", { format: "webp" }), op("quality-web", "quality", { quality: 82 }), op("strip-web", "stripMetadata")] },
  { id: "private", name: "Private photo share", description: "Keep format and remove all metadata", builtin: true, operations: [op("strip-private", "stripMetadata")] },
  { id: "discord", name: "Discord video", description: "720p H.264 video without audio", builtin: true, operations: [op("resize-discord", "resize", { width: 1280, height: 720, fit: "contain" }), op("quality-discord", "quality", { quality: 68 }), op("audio-discord", "removeAudio"), op("format-discord", "format", { format: "mp4" })] }
];
