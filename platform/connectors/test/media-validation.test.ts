import { describe, expect, it } from "vitest";
import { parseChannelInboundMedia } from "../src/media/inbound.js";
import {
  MAX_AUDIO_BYTES,
  MAX_IMAGE_BYTES,
  MAX_TEXT_DOCUMENT_BYTES,
  audioMime,
  documentMime,
  imageMime,
  mediaByteLimit,
} from "../src/media/validation.js";

describe("channel media validation", () => {
  it("normalizes supported document and audio aliases", () => {
    expect(documentMime("notes.MD", "application/octet-stream")).toBe("text/markdown");
    expect(documentMime("malware.exe", "application/octet-stream")).toBeNull();
    expect(documentMime(undefined, "application/pdf; charset=binary")).toBe("application/pdf");
    expect(audioMime("voice.opus", "application/octet-stream")).toBe("audio/ogg");
    expect(audioMime(undefined, "audio/x-wav")).toBe("audio/wav");
    expect(audioMime(undefined, "audio/x-m4a; codecs=aac")).toBe("audio/mp4");
    expect(imageMime("image/svg+xml")).toBeNull();
  });

  it("uses media-specific byte limits", () => {
    expect(mediaByteLimit("image", "image/jpeg")).toBe(MAX_IMAGE_BYTES);
    expect(mediaByteLimit("audio", "audio/ogg")).toBe(MAX_AUDIO_BYTES);
    expect(mediaByteLimit("document", "text/plain")).toBe(MAX_TEXT_DOCUMENT_BYTES);
  });

  it("validates activity media references without accepting bytes", () => {
    expect(
      parseChannelInboundMedia({ fileId: "file-1", kind: "image", mime: "IMAGE/JPEG", name: null, byteSize: null }),
    ).toEqual({ fileId: "file-1", kind: "image", mime: "image/jpeg" });
    expect(
      parseChannelInboundMedia({ fileId: "file-2", kind: "audio", mime: "audio/opus", name: "clip.opus", byteSize: 10 }),
    ).toEqual({ fileId: "file-2", kind: "audio", mime: "audio/ogg", name: "clip.opus", byteSize: 10 });
    expect(() => parseChannelInboundMedia({ fileId: "", kind: "image", mime: "image/jpeg" })).toThrow(
      /fileId/,
    );
    expect(() => parseChannelInboundMedia({ fileId: "f", kind: "video", mime: "video/mp4" })).toThrow(
      /kind is invalid/,
    );
    expect(() => parseChannelInboundMedia({ fileId: "f", kind: "image", mime: "image/svg+xml" })).toThrow(
      /unsupported image MIME/,
    );
    expect(() =>
      parseChannelInboundMedia({ fileId: "f", kind: "document", mime: "text/plain", byteSize: MAX_TEXT_DOCUMENT_BYTES + 1 }),
    ).toThrow(RangeError);
  });
});
