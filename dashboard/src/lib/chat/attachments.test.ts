import { describe, expect, it } from "vitest";
import {
  chipMeta,
  dragHasFiles,
  formatBytes,
  isImageType,
  isLongPaste,
  toBase64,
  typeTag,
} from "./attachments";

describe("attachments", () => {
  it("formats sizes the way chips show them", () => {
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(4 * 1024)).toBe("4 KB");
    expect(formatBytes(1100)).toBe("1 KB");
    expect(formatBytes(1.2 * 1024 * 1024)).toBe("1.2 MB");
    expect(formatBytes(212 * 1024 * 1024)).toBe("212 MB");
  });

  it("tags files by extension", () => {
    expect(typeTag("theme-tokens.css")).toBe("CSS");
    expect(typeTag("ui-spec.pdf")).toBe("PDF");
    expect(typeTag("Makefile")).toBe("FILE");
    expect(typeTag(".env")).toBe("FILE");
    expect(typeTag("notes.markdown")).toBe("MARK");
  });

  it("describes pasted text by lines and files by size", () => {
    const base = { name: "a", path: "/a", bytes: 4096, lines: null };
    expect(chipMeta({ ...base, kind: "file" })).toBe("4 KB");
    expect(chipMeta({ ...base, kind: "text", lines: 84 })).toBe("84 lines");
    expect(chipMeta({ ...base, kind: "text", lines: 1 })).toBe("1 line");
  });

  it("treats long pastes as attachments", () => {
    expect(isLongPaste("short")).toBe(false);
    expect(isLongPaste("a\n".repeat(40))).toBe(true);
    expect(isLongPaste("a\n".repeat(39))).toBe(false);
    expect(isLongPaste("x".repeat(2000))).toBe(true);
  });

  it("knows previewable images and file drags", () => {
    expect(isImageType("image/png")).toBe(true);
    expect(isImageType("image/svg+xml")).toBe(false);
    expect(dragHasFiles(["Files"])).toBe(true);
    expect(dragHasFiles(["text/plain"])).toBe(false);
  });

  it("encodes base64 like btoa", () => {
    expect(toBase64(new TextEncoder().encode("hello"))).toBe("aGVsbG8=");
    expect(toBase64(new Uint8Array(0))).toBe("");
  });
});
