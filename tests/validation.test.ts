import { describe, expect, it } from "vitest";
import { validateRecipe } from "../src/validation";
import type { MediaFile, Operation } from "../src/types";

const image: MediaFile = { path: "/photo.jpg", name: "photo.jpg", kind: "image", size: 10 };
const operation = (kind: Operation["kind"], extra: Partial<Operation> = {}): Operation => ({ id: kind, kind, enabled: true, ...extra });

describe("validateRecipe", () => {
  it("accepts a bounded image recipe", () => {
    expect(validateRecipe([operation("resize", { width: 2048, height: 2048 }), operation("quality", { quality: 82 })], [image])).toEqual([]);
  });
  it("rejects unsafe dimensions and duplicate format decisions", () => {
    const errors = validateRecipe([operation("resize", { width: 0, height: 99_999 }), operation("format", { format: "jpg" }), operation("format", { format: "webp" })], [image]);
    expect(errors).toContain("Resize dimensions must be between 1 and 16,384 pixels.");
    expect(errors).toContain("Use only one output-format step.");
  });
  it("rejects video-only work for an image batch", () => {
    expect(validateRecipe([operation("removeAudio")], [image])).toContain("Video-only steps do not apply to this image batch.");
  });
  it("rejects operation names outside the model", () => {
    expect(validateRecipe([operation("shell" as Operation["kind"])], [])).toContain("Unsupported operation: shell.");
  });
});
