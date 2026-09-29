/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "../../i18n";
import SystemPromptCachePage from "./SystemPromptCachePage";

const apiMocks = vi.hoisted(() => ({
  fetchPromptCacheMaterializationStatus: vi.fn(),
  updatePromptCacheMaterializationControl: vi.fn(),
}));

vi.mock("../../lib/api", async () => {
  const actual = await vi.importActual<typeof import("../../lib/api")>("../../lib/api");
  return {
    ...actual,
    fetchPromptCacheMaterializationStatus: apiMocks.fetchPromptCacheMaterializationStatus,
    updatePromptCacheMaterializationControl: apiMocks.updatePromptCacheMaterializationControl,
  };
});

let host: HTMLDivElement | null = null;
let root: Root | null = null;

const response = {
  enabled: true,
  phase: "stats_rebuild",
  totalKeys: 400,
  completedKeys: 128,
  queuePending: 3,
  progressPercent: 32,
  estimatedRemainingMs: 7_500,
  sourceMaxInvocationId: 1_024,
  updatedAt: "2026-09-29T04:16:46.000Z",
  lastStartedAt: "2026-09-29T04:16:45.000Z",
  lastFinishedAt: "2026-09-29T04:16:46.000Z",
  lastStatus: "success",
  recentRuns: [
    {
      id: 7,
      startedAt: "2026-09-29T04:16:45.000Z",
      finishedAt: "2026-09-29T04:16:46.000Z",
      phase: "stats_rebuild",
      status: "success",
      scanned: 64,
      updated: 64,
      batchCount: 1,
      lastBatchSize: 64,
      maxBatchSize: 64,
      batchElapsedMs: 94,
      durationMs: 120,
    },
  ],
};

function renderPage() {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(
      <I18nProvider initialLocale="zh">
        <SystemPromptCachePage />
      </I18nProvider>,
    );
  });
}

async function flushEffects() {
  await act(async () => {
    await Promise.resolve();
  });
}

describe("SystemPromptCachePage", () => {
  beforeEach(() => {
    apiMocks.fetchPromptCacheMaterializationStatus.mockResolvedValue(response);
    apiMocks.updatePromptCacheMaterializationControl.mockResolvedValue({
      ...response,
      enabled: false,
      lastStatus: "disabled",
    });
  });

  afterEach(() => {
    act(() => {
      root?.unmount();
    });
    host?.remove();
    host = null;
    root = null;
    apiMocks.fetchPromptCacheMaterializationStatus.mockReset();
    apiMocks.updatePromptCacheMaterializationControl.mockReset();
  });

  it("shows durable progress and recent run details", async () => {
    renderPage();
    await flushEffects();

    expect(host?.textContent ?? "").toContain("Prompt-cache 物化");
    expect(host?.textContent ?? "").toContain("400");
    expect(host?.textContent ?? "").toContain("128");
    expect(host?.textContent ?? "").toContain("32.0%");
    expect(host?.textContent ?? "").toContain("64");
    expect(host?.textContent ?? "").toContain("成功");
    expect(host?.querySelector('[role="progressbar"]')?.getAttribute("aria-valuenow")).toBe("32");
  });

  it("pauses the task through the control switch", async () => {
    renderPage();
    await flushEffects();

    const toggle = host?.querySelector('[role="switch"]');
    expect(toggle).toBeTruthy();
    await act(async () => {
      toggle?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await Promise.resolve();
    });

    expect(apiMocks.updatePromptCacheMaterializationControl).toHaveBeenCalledWith(false);
  });
});
