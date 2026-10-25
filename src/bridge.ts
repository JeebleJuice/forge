import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { BUILTIN_PRESETS } from "./presets";
import type { JobProgress, MediaFile, PersistedState, Preset, ProcessRequest, ProcessSummary, ToolStatus } from "./types";

export const isNative = () => "__TAURI_INTERNALS__" in window;

const demoFiles: MediaFile[] = [
  { path: "/demo/aurora-morning.jpg", name: "aurora-morning.jpg", kind: "image", size: 8_640_000, width: 4032, height: 3024, format: "JPEG" },
  { path: "/demo/coastline.png", name: "coastline.png", kind: "image", size: 12_480_000, width: 2560, height: 1707, format: "PNG" },
  { path: "/demo/studio-tour.mov", name: "studio-tour.mov", kind: "video", size: 184_000_000, width: 1920, height: 1080, duration: 84, format: "MOV", hasAudio: true }
];

export async function inspectPaths(paths: string[]): Promise<MediaFile[]> {
  if (isNative()) return invoke("inspect_paths", { paths });
  return paths.length ? demoFiles : [];
}

export async function pickFiles(): Promise<MediaFile[]> {
  if (!isNative()) return demoFiles;
  const paths = await invoke<string[]>("pick_files");
  return inspectPaths(paths);
}

export async function pickOutputDirectory(): Promise<string | null> {
  if (!isNative()) return "/home/you/Pictures/Forge exports";
  return invoke("pick_output_directory");
}

export async function loadState(): Promise<PersistedState> {
  if (isNative()) return invoke("load_state");
  const stored = localStorage.getItem("forge-state");
  return stored ? JSON.parse(stored) : { presets: [], history: [] };
}

export async function saveState(state: PersistedState): Promise<void> {
  if (isNative()) return invoke("save_state", { persisted: state });
  localStorage.setItem("forge-state", JSON.stringify(state));
}

export async function toolStatus(): Promise<ToolStatus> {
  if (isNative()) return invoke("tool_status");
  return { ffmpeg: true, ffprobe: true, imagemagick: true };
}

export async function startJob(request: ProcessRequest): Promise<ProcessSummary> {
  if (isNative()) return invoke("process_job", { request });
  for (let index = 0; index < request.files.length; index++) {
    await new Promise((resolve) => setTimeout(resolve, 450));
    window.dispatchEvent(new CustomEvent("forge-demo-progress", { detail: { jobId: request.jobId, index, total: request.files.length, path: request.files[index].path, status: "completed", outputPath: `${request.outputDir}/${request.files[index].name}`, outputSize: Math.round(request.files[index].size * 0.42) } }));
  }
  return { completed: request.files.length, failed: 0 };
}

export async function cancelJob(jobId: string): Promise<void> {
  if (isNative()) return invoke("cancel_job", { jobId });
}

export async function revealPath(path: string): Promise<void> {
  if (isNative()) return invoke("reveal_path", { path });
}

export async function onProgress(handler: (progress: JobProgress) => void): Promise<UnlistenFn> {
  if (isNative()) return listen<JobProgress>("job-progress", (event) => handler(event.payload));
  const listener = (event: Event) => handler((event as CustomEvent<JobProgress>).detail);
  window.addEventListener("forge-demo-progress", listener);
  return () => window.removeEventListener("forge-demo-progress", listener);
}

export async function onFileDrop(handler: (paths: string[]) => void): Promise<UnlistenFn> {
  if (!isNative()) return () => undefined;
  return listen<{ paths: string[] }>("tauri://drag-drop", (event) => handler(event.payload.paths));
}

export async function readPreview(path: string): Promise<{ bytes: number[]; mime: string } | null> {
  if (!isNative() || path.startsWith("/demo/")) return null;
  return invoke("read_preview", { path });
}

export async function renderPreview(path: string, operations: ProcessRequest["operations"]): Promise<{ bytes: number[]; mime: string } | null> {
  if (!isNative() || path.startsWith("/demo/")) return null;
  return invoke("render_preview", { path, operations });
}

export async function exportPreset(preset: Preset): Promise<boolean> {
  if (isNative()) return invoke("export_preset", { preset });
  const link = document.createElement("a");
  link.href = URL.createObjectURL(new Blob([JSON.stringify(preset, null, 2)], { type: "application/json" }));
  link.download = `${preset.name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || "forge-preset"}.forge.json`;
  link.click(); URL.revokeObjectURL(link.href); return true;
}

export async function importPreset(): Promise<Preset | null> {
  if (isNative()) return invoke("import_preset");
  return new Promise((resolve, reject) => {
    const input = document.createElement("input"); input.type = "file"; input.accept = ".json,.forge.json";
    input.onchange = async () => { try { resolve(input.files?.[0] ? JSON.parse(await input.files[0].text()) : null); } catch (error) { reject(error); } };
    input.click();
  });
}

export const defaultState: PersistedState = { presets: BUILTIN_PRESETS, history: [] };
