import type { MediaFile, Operation } from "./types";

export function formatBytes(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  const unit = Math.min(Math.floor(Math.log(value) / Math.log(1024)), units.length - 1);
  const size = value / 1024 ** unit;
  return `${size.toFixed(unit === 0 || size >= 10 ? 0 : 1)} ${units[unit]}`;
}

export function estimateOutput(file: MediaFile, operations: Operation[]): number {
  let ratio = 1;
  const resize = operations.find((item) => item.enabled && item.kind === "resize");
  if (resize && file.width && file.height && resize.width && resize.height) {
    const scale = Math.min(1, resize.width / file.width, resize.height / file.height);
    ratio *= scale * scale;
  }
  const quality = operations.find((item) => item.enabled && item.kind === "quality")?.quality ?? 82;
  ratio *= file.kind === "video" ? 0.25 + quality / 160 : 0.18 + quality / 130;
  if (operations.some((item) => item.enabled && item.kind === "removeAudio")) ratio *= 0.9;
  return Math.max(1024, Math.round(file.size * Math.min(1, ratio)));
}

export const basename = (path: string) => path.split(/[\\/]/).pop() || path;
