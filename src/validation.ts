import type { MediaFile, Operation } from "./types";

export function validateRecipe(operations: Operation[], files: MediaFile[]): string[] {
  const enabled = operations.filter((operation) => operation.enabled);
  const errors: string[] = [];
  const allowed = new Set(["resize", "format", "quality", "stripMetadata", "trim", "removeAudio", "rotate"]);
  if (!enabled.length) errors.push("Enable at least one processing step.");
  if (enabled.length > 20) errors.push("A recipe can contain at most 20 enabled steps.");
  if (enabled.filter((operation) => operation.kind === "format").length > 1) errors.push("Use only one output-format step.");
  for (const operation of enabled) {
    if (!allowed.has(operation.kind)) errors.push(`Unsupported operation: ${String(operation.kind)}.`);
    if (operation.kind === "resize" && (!operation.width || !operation.height || operation.width < 1 || operation.height < 1 || operation.width > 16_384 || operation.height > 16_384)) {
      errors.push("Resize dimensions must be between 1 and 16,384 pixels.");
    }
    if (operation.kind === "quality" && (!operation.quality || operation.quality < 1 || operation.quality > 100)) {
      errors.push("Quality must be between 1 and 100.");
    }
    if (operation.kind === "trim" && ((operation.start ?? 0) < 0 || operation.end === undefined || operation.end <= (operation.start ?? 0))) {
      errors.push("Trim must end after its non-negative start time.");
    }
  }
  if (files.length && files.every((file) => file.kind === "image") && enabled.some((operation) => operation.kind === "trim" || operation.kind === "removeAudio")) {
    errors.push("Video-only steps do not apply to this image batch.");
  }
  return [...new Set(errors)];
}
