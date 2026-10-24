import { describe, expect, it } from "vitest";
import { estimateOutput, formatBytes } from "../src/format";

describe("formatBytes", () => {
  it("formats human-sized values", () => {
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(formatBytes(10 * 1024 * 1024)).toBe("10 MB");
  });
});

describe("estimateOutput", () => {
  it("accounts for resize area", () => {
    const file = { path: "/photo.jpg", name: "photo.jpg", kind: "image" as const, size: 8_000_000, width: 4000, height: 3000 };
    const full = estimateOutput(file, []);
    const small = estimateOutput(file, [{ id: "r", kind: "resize", enabled: true, width: 1000, height: 1000 }]);
    expect(small).toBeLessThan(full);
  });
});
