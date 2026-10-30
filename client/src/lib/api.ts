export type ProcState = "starting" | "ready" | "busy" | "restarting" | "failed";

export interface Lost {
  at_ms: number;
  exit_code: number | null;
  unsaved: boolean;
  file: string | null;
  message: string;
}

export interface Status {
  state: ProcState;
  pid: number | null;
  version: string | null;
  device_type: string | null;
  devices: string[];
  file: string | null;
  unsaved: boolean;
  busy_with: string | null;
  restarts: number;
  last_error: string | null;
  lost: Lost | null;
}

export type Phase = "rendering" | "encoding" | "done" | "cancelled" | "failed";

export interface Job {
  id: string;
  name: string;
  created_ms: number;
  frame_start: number;
  frame_end: number;
  done_frames: number;
  phase: Phase;
  cancel_requested: boolean;
  error: string | null;
  outputs: string[];
}

export type Kind =
  | "image"
  | "video"
  | "sequence"
  | "blend"
  | "script"
  | "audio"
  | "model"
  | "other";

export interface Entry {
  path: string;
  kind: Kind;
  size: number;
  modified_ms: number;
  frames?: number;
  cover?: string;
}

export type Dir = "renders" | "scenes" | "scripts" | "assets";

async function requestJson<T>(
  url: string,
  init?: RequestInit & { signal?: AbortSignal },
): Promise<T> {
  const response = await fetch(url, init);
  if (!response.ok) {
    throw new Error(`HTTP ${response.status}`);
  }
  return (await response.json()) as T;
}

export function fetchFiles(dir: Dir, signal?: AbortSignal): Promise<Entry[]> {
  return requestJson(`/api/files/${dir}`, { signal });
}

export function cancelJob(id: string): Promise<Job> {
  return requestJson(`/api/jobs/${encodeURIComponent(id)}/cancel`, {
    method: "POST",
  });
}
