import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import Home from "./Home.svelte";
import type { Entry, Job, Status } from "../lib/api";

class FakeEventSource {
  static last: FakeEventSource | undefined;
  onmessage: ((event: MessageEvent) => void) | null = null;
  onopen: (() => void) | null = null;
  closed = false;
  constructor(public url: string) {
    FakeEventSource.last = this;
  }
  emit(type: string, data?: unknown) {
    this.onmessage?.(
      new MessageEvent("message", { data: JSON.stringify({ type, data }) }),
    );
  }
  close() {
    this.closed = true;
  }
}

const STATUS: Status = {
  state: "ready",
  pid: 1,
  version: "5.2.2 LTS",
  device_type: "OPTIX",
  devices: ["NVIDIA GeForce RTX 5060 Ti"],
  file: "scenes/cube.blend",
  unsaved: true,
  busy_with: null,
  restarts: 0,
  last_error: null,
  lost: null,
};

const JOB: Job = {
  id: "j1",
  name: "turntable",
  created_ms: 0,
  frame_start: 1,
  frame_end: 48,
  done_frames: 12,
  phase: "rendering",
  cancel_requested: false,
  error: null,
  outputs: [],
};

function image(path: string, modified_ms: number): Entry {
  return { path, kind: "image", size: 2048, modified_ms };
}

let files: Record<string, Entry[]>;
let fetchMock: ReturnType<typeof vi.fn<typeof fetch>>;

beforeEach(() => {
  files = {
    renders: [image("renders/first.png", 1)],
    scenes: [
      {
        path: "scenes/cube.blend",
        kind: "blend",
        size: 900000,
        modified_ms: 1,
      },
    ],
    scripts: [],
    assets: [],
  };
  fetchMock = vi.fn<typeof fetch>().mockImplementation((input, init) => {
    const url = String(input);
    if (init?.method === "POST") {
      return Promise.resolve(
        new Response(JSON.stringify({ ...JOB, cancel_requested: true })),
      );
    }
    const dir = url.replace("/api/files/", "");
    return Promise.resolve(new Response(JSON.stringify(files[dir] ?? [])));
  });
  vi.stubGlobal("fetch", fetchMock);
  vi.stubGlobal("EventSource", FakeEventSource);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("Home", () => {
  it("adds a new render without a reload when the base camp changes", async () => {
    render(Home);
    await waitFor(() => expect(screen.getByText("first.png")).toBeTruthy());
    expect(FakeEventSource.last?.url).toBe("/api/events");

    files.renders = [image("renders/second.png", 2), ...files.renders];
    FakeEventSource.last?.emit("files");

    await waitFor(() => expect(screen.getByText("second.png")).toBeTruthy());
    const names = [...document.querySelectorAll(".render .name")].map(
      (n) => n.textContent,
    );
    expect(names).toEqual(["second.png", "first.png"]);
  });

  it("shows the Blender process and the open file", async () => {
    render(Home);
    FakeEventSource.last?.emit("status", STATUS);
    await waitFor(() => expect(screen.getByText("待機中")).toBeTruthy());
    expect(screen.getByText("scenes/cube.blend")).toBeTruthy();
    expect(screen.getByText("未保存")).toBeTruthy();
    expect(screen.getByText(/Cycles NVIDIA GeForce RTX 5060 Ti/)).toBeTruthy();
  });

  it("explains a crash that lost the unsaved scene", async () => {
    render(Home);
    FakeEventSource.last?.emit("status", {
      ...STATUS,
      restarts: 1,
      unsaved: false,
      lost: {
        at_ms: Date.now(),
        exit_code: 1,
        unsaved: true,
        file: null,
        message: "Blender exited",
      },
    });
    await waitFor(() =>
      expect(screen.getByRole("alert").textContent).toContain(
        "未保存の scene は失われました",
      ),
    );
  });

  it("shows job progress and cancels from the browser", async () => {
    render(Home);
    FakeEventSource.last?.emit("jobs", [JOB]);
    const bar = await screen.findByRole("progressbar");
    expect(bar.getAttribute("aria-valuenow")).toBe("25");
    expect(screen.getByText("12 / 48")).toBeTruthy();

    await fireEvent.click(
      screen.getByRole("button", { name: "turntable を取り消す" }),
    );
    expect(fetchMock).toHaveBeenCalledWith("/api/jobs/j1/cancel", {
      method: "POST",
    });

    FakeEventSource.last?.emit("jobs", [
      {
        ...JOB,
        phase: "done",
        done_frames: 48,
        outputs: ["renders/turntable.mp4"],
      },
    ]);
    await waitFor(() => expect(screen.getByText("完了")).toBeTruthy());
    expect(
      screen.queryByRole("button", { name: "turntable を取り消す" }),
    ).toBeNull();
  });

  it("lists other base camp files with download links", async () => {
    render(Home);
    const link = await screen.findByRole("link", {
      name: "cube.blend をダウンロード",
    });
    expect(link.getAttribute("href")).toBe(
      "/files/scenes/cube.blend?download=1",
    );
    await fireEvent.click(screen.getByRole("tab", { name: /assets/ }));
    await waitFor(() =>
      expect(screen.getByText("ファイルがありません")).toBeTruthy(),
    );
  });

  it("closes the event stream when it leaves the page", async () => {
    const { unmount } = render(Home);
    const source = FakeEventSource.last;
    unmount();
    expect(source?.closed).toBe(true);
  });
});
