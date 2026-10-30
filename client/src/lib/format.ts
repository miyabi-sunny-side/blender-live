import type { Job, ProcState } from "./api";

const PROCESS_LABELS: Record<ProcState, string> = {
  ready: "待機中",
  busy: "処理中",
  starting: "起動中",
  restarting: "再起動中",
  failed: "停止",
};

export function processLabel(state: ProcState): string {
  return PROCESS_LABELS[state];
}

export function phaseLabel(job: Job): string {
  switch (job.phase) {
    case "rendering":
      return job.cancel_requested ? "取り消し中" : "書き出し中";
    case "encoding":
      return "MP4 に変換中";
    case "done":
      return "完了";
    case "cancelled":
      return "取り消し";
    case "failed":
      return "失敗";
  }
}

export function totalFrames(job: Job): number {
  return job.frame_end - job.frame_start + 1;
}

export function progressPercent(job: Job): number {
  if (job.phase === "encoding" || job.phase === "done") {
    return 100;
  }
  return Math.floor((job.done_frames / totalFrames(job)) * 100);
}

export function formatSize(bytes: number): string {
  const units = ["KB", "MB", "GB"];
  if (bytes < 1024) {
    return `${bytes} B`;
  }
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(1)} ${units[unit]}`;
}

export function formatTime(
  ms: number,
  nowMs: number,
  timeZone?: string,
): string {
  const day = new Intl.DateTimeFormat("en-CA", {
    timeZone,
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
  });
  const time = new Intl.DateTimeFormat("ja-JP", {
    timeZone,
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23",
  }).format(ms);
  if (day.format(ms) === day.format(nowMs)) {
    return time;
  }
  const date = new Intl.DateTimeFormat("ja-JP", {
    timeZone,
    month: "numeric",
    day: "numeric",
  }).format(ms);
  return `${date} ${time}`;
}

function encodePath(path: string): string {
  return path.split("/").map(encodeURIComponent).join("/");
}

export function fileUrl(path: string, download = false): string {
  return `/files/${encodePath(path)}${download ? "?download=1" : ""}`;
}

export function viewHref(path: string): string {
  return `/view/${encodePath(path)}`;
}

export function basename(path: string): string {
  return path.slice(path.lastIndexOf("/") + 1);
}

export function mediaKind(path: string): "image" | "video" | "other" {
  if (/\.(png|jpe?g|webp|gif)$/i.test(path)) {
    return "image";
  }
  if (/\.(mp4|webm|mov)$/i.test(path)) {
    return "video";
  }
  return "other";
}
