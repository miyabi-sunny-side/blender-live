import { describe, expect, it } from "vitest";

import {
  basename,
  fileUrl,
  formatSize,
  formatTime,
  phaseLabel,
  processLabel,
  progressPercent,
  viewHref,
} from "./format";
import type { Job } from "./api";

function job(patch: Partial<Job>): Job {
  return {
    id: "a",
    name: "clip",
    created_ms: 0,
    frame_start: 1,
    frame_end: 48,
    done_frames: 0,
    phase: "rendering",
    cancel_requested: false,
    error: null,
    outputs: [],
    ...patch,
  };
}

describe("labels", () => {
  it("names every process state in Japanese", () => {
    expect(processLabel("ready")).toBe("待機中");
    expect(processLabel("busy")).toBe("処理中");
    expect(processLabel("starting")).toBe("起動中");
    expect(processLabel("restarting")).toBe("再起動中");
    expect(processLabel("failed")).toBe("停止");
  });

  it("names every job phase", () => {
    expect(phaseLabel(job({}))).toBe("書き出し中");
    expect(phaseLabel(job({ cancel_requested: true }))).toBe("取り消し中");
    expect(phaseLabel(job({ phase: "encoding" }))).toBe("MP4 に変換中");
    expect(phaseLabel(job({ phase: "done" }))).toBe("完了");
    expect(phaseLabel(job({ phase: "cancelled" }))).toBe("取り消し");
    expect(phaseLabel(job({ phase: "failed" }))).toBe("失敗");
  });
});

describe("progressPercent", () => {
  it("counts rendered frames of the inclusive range", () => {
    expect(progressPercent(job({ done_frames: 0 }))).toBe(0);
    expect(progressPercent(job({ done_frames: 12 }))).toBe(25);
    expect(progressPercent(job({ done_frames: 47 }))).toBe(97);
    expect(progressPercent(job({ phase: "encoding", done_frames: 48 }))).toBe(
      100,
    );
    expect(progressPercent(job({ phase: "cancelled", done_frames: 6 }))).toBe(
      12,
    );
  });
});

describe("formatSize", () => {
  it("uses binary units with one decimal", () => {
    expect(formatSize(512)).toBe("512 B");
    expect(formatSize(1536)).toBe("1.5 KB");
    expect(formatSize(12.3 * 1024 * 1024)).toBe("12.3 MB");
    expect(formatSize(2 * 1024 ** 3)).toBe("2.0 GB");
  });
});

describe("formatTime", () => {
  const now = Date.UTC(2026, 9, 30, 8, 0, 0);
  it("shows only the time for today and the date otherwise", () => {
    expect(formatTime(Date.UTC(2026, 9, 30, 7, 5, 9), now, "Asia/Tokyo")).toBe(
      "16:05",
    );
    expect(formatTime(Date.UTC(2026, 9, 28, 1, 2, 0), now, "Asia/Tokyo")).toBe(
      "10/28 10:02",
    );
  });
});

describe("urls", () => {
  it("encodes each path segment", () => {
    expect(fileUrl("renders/モカ 正面.png")).toBe(
      "/files/renders/%E3%83%A2%E3%82%AB%20%E6%AD%A3%E9%9D%A2.png",
    );
    expect(fileUrl("scenes/a#1.blend", true)).toBe(
      "/files/scenes/a%231.blend?download=1",
    );
    expect(viewHref("renders/a b.mp4")).toBe("/view/renders/a%20b.mp4");
    expect(basename("assets/moca/宮舞モカ.pmx")).toBe("宮舞モカ.pmx");
  });
});

describe("mediaKind", () => {
  it("tells images and videos apart by extension", async () => {
    const { mediaKind } = await import("./format");
    expect(mediaKind("renders/a.PNG")).toBe("image");
    expect(mediaKind("renders/clip/frame_0001.png")).toBe("image");
    expect(mediaKind("renders/a.mp4")).toBe("video");
    expect(mediaKind("renders/a.webm")).toBe("video");
    expect(mediaKind("scenes/a.blend")).toBe("other");
  });
});
