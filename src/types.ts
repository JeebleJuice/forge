export type MediaKind = "image" | "video" | "unknown";
export type JobStatus = "ready" | "running" | "completed" | "failed" | "cancelled";
export type OperationKind = "resize" | "format" | "quality" | "stripMetadata" | "trim" | "removeAudio" | "rotate";

export interface MediaFile {
  path: string;
  name: string;
  kind: MediaKind;
  size: number;
  width?: number;
  height?: number;
  duration?: number;
  format?: string;
  hasAudio?: boolean;
  status?: JobStatus;
  outputPath?: string;
  outputSize?: number;
  error?: string;
}

export interface Operation {
  id: string;
  kind: OperationKind;
  enabled: boolean;
  width?: number;
  height?: number;
  fit?: "contain" | "cover" | "exact";
  format?: "original" | "jpg" | "png" | "webp" | "avif" | "mp4" | "webm";
  quality?: number;
  start?: number;
  end?: number;
  degrees?: 90 | 180 | 270;
}

export interface Preset { id: string; name: string; description: string; operations: Operation[]; builtin?: boolean; }
export interface HistoryEntry { id: string; startedAt: string; finishedAt: string; status: JobStatus; inputCount: number; outputCount: number; bytesBefore: number; bytesAfter: number; outputDir: string; errors: string[]; }
export interface PersistedState { presets: Preset[]; history: HistoryEntry[]; outputDir?: string; }
export interface ToolStatus { ffmpeg: boolean; ffprobe: boolean; imagemagick: boolean; }
export interface JobProgress { jobId: string; index: number; total: number; path: string; status: JobStatus; fileProgress?: number; outputPath?: string; outputSize?: number; error?: string; }
export interface ProcessRequest { jobId: string; files: MediaFile[]; operations: Operation[]; outputDir: string; }
export interface ProcessSummary { completed: number; failed: number; }
