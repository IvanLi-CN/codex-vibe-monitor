/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { ObservabilityCapabilities } from "./api";
import { BrowserObservability, usePageObservation } from "./browserObservability";
import {
  configureBrowserObservation,
  flushBrowserObservations,
  observeBrowser,
} from "./observation-events";

const mocks = vi.hoisted(() => ({ capabilities: vi.fn() }));
vi.mock("./api", () => ({
  fetchObservabilityCapabilities: mocks.capabilities,
  postBrowserObservations: vi.fn().mockResolvedValue(undefined),
}));
let host: HTMLDivElement;
let root: Root;
let now: number;
let frames: Map<number, FrameRequestCallback>;
let frameId: number;
beforeEach(() => {
  configureBrowserObservation(null, false);
  now = 100;
  frames = new Map();
  frameId = 0;
  vi.spyOn(performance, "now").mockImplementation(() => now);
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    frames.set(++frameId, callback);
    return frameId;
  });
  vi.stubGlobal("cancelAnimationFrame", (id: number) => frames.delete(id));
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  configureBrowserObservation(null, false);
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});
function Page({ data }: { data: unknown }): null {
  usePageObservation("dashboard", data);
  return null;
}
async function frame(): Promise<void> {
  const scheduled = [...frames.values()];
  frames.clear();
  await act(async () => {
    for (const callback of scheduled) callback(now);
  });
}
it("preserves actual ready and paint timings when capability discovery is late", async () => {
  await act(async () => root.render(<Page data={null} />));
  now = 120;
  await act(async () => root.render(<Page data="ready" />));
  now = 130;
  await frame();
  now = 140;
  await frame();
  now = 1000;
  await act(async () => configureBrowserObservation("dashboard", true));
  const send = vi.fn().mockResolvedValue(undefined);
  await flushBrowserObservations(send, 10_000_000);
  expect(send.mock.calls[0][0]).toEqual([
    expect.objectContaining({ kind: "data_ready", valueSeconds: 0.02, page: "dashboard" }),
    expect.objectContaining({ kind: "update_to_paint", valueSeconds: 0.02, page: "dashboard" }),
  ]);
  expect(frames.size).toBe(0);
});
it("activates the current router page rather than the initial page or window pathname", async () => {
  let resolve!: (capabilities: ObservabilityCapabilities) => void;
  mocks.capabilities.mockReturnValue(
    new Promise<ObservabilityCapabilities>((done) => {
      resolve = done;
    }),
  );
  await act(async () => root.render(<BrowserObservability pathname="/dashboard" />));
  await act(async () => root.render(<BrowserObservability pathname="/records" />));
  await act(async () =>
    resolve({
      enabled: true,
      state: "enabled",
      grafanaPublicUrl: null,
      grafanaConnectivity: "unknown",
      hotpath: true,
      dashboards: [],
      datasourceUid: "cvm-prometheus",
      variables: [],
    }),
  );
  observeBrowser("api_request", 0.5);
  const send = vi.fn().mockResolvedValue(undefined);
  await flushBrowserObservations(send, 10_100_000);
  expect(send.mock.calls[0][0]).toContainEqual(
    expect.objectContaining({ kind: "api_request", page: "records" }),
  );
});
