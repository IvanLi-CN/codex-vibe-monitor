/** @vitest-environment jsdom */

import { waitFor, within } from "@testing-library/dom";
import userEvent from "@testing-library/user-event";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "../../i18n";
import SystemTasksPage from "./SystemTasksPage";

const apiMocks = vi.hoisted(() => ({
  fetchManagedTasks: vi.fn(),
  fetchManagedTaskRuntime: vi.fn(),
  fetchManagedTaskTimeline: vi.fn(),
}));
vi.mock("../../lib/api", async () => ({
  ...(await vi.importActual<typeof import("../../lib/api")>("../../lib/api")),
  fetchManagedTasks: apiMocks.fetchManagedTasks,
  fetchManagedTaskRuntime: apiMocks.fetchManagedTaskRuntime,
  fetchManagedTaskTimeline: apiMocks.fetchManagedTaskTimeline,
}));

let host: HTMLDivElement | null = null;
let root: Root | null = null;

function renderPage() {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() =>
    root?.render(
      <MemoryRouter>
        <I18nProvider>
          <SystemTasksPage />
        </I18nProvider>
      </MemoryRouter>,
    ),
  );
}

async function flushEffects() {
  await act(async () => {
    await Promise.resolve();
  });
}

describe("SystemTasksPage", () => {
  beforeEach(() => {
    HTMLElement.prototype.scrollIntoView = () => undefined;
    HTMLElement.prototype.hasPointerCapture = () => false;
    HTMLElement.prototype.setPointerCapture = () => undefined;
    HTMLElement.prototype.releasePointerCapture = () => undefined;
    apiMocks.fetchManagedTaskRuntime.mockResolvedValue({
      observedAt: "2026-10-01T00:00:00.000Z",
      activeRuns: [
        {
          executionId: 42,
          executionUid: "run-42",
          taskKey: "retention_archive",
          title: "数据保留与归档",
          activeChildTaskKey: null,
          activeChildTitle: null,
          triggerKind: "interval",
          phase: "processing",
          executionClass: "maintenance_retention",
          startedAt: "2026-10-01T00:00:00.000Z",
          elapsedMs: 12_000,
        },
      ],
      queuedRuns: [
        {
          runId: 43,
          taskKey: "retention_archive",
          title: "数据保留与归档",
          triggerKind: "manual",
          requestedAt: "2026-10-01T00:00:01.000Z",
          waitingMs: 5_000,
          position: 1,
        },
      ],
      queuedRunsAvailable: true,
      admissionWaits: [
        {
          id: "wait-44",
          taskKey: "retention_archive",
          title: "数据保留与归档",
          reason: "pressure_cooldown",
          startedAt: "2026-10-01T00:00:00.000Z",
          waitingMs: 8_000,
          retryAt: null,
        },
      ],
      admissionWaitsAvailable: true,
    });
    apiMocks.fetchManagedTaskTimeline.mockResolvedValue({
      observedAt: "2026-10-01T00:00:02.000Z",
      windowStart: "2026-09-30T00:00:02.000Z",
      windowEnd: "2026-10-01T00:00:02.000Z",
      watermark: 1,
      segments: [],
      coverage: [],
      nextCursor: null,
      resetRequired: false,
    });
    apiMocks.fetchManagedTasks.mockResolvedValue([
      {
        taskKey: "retention_archive",
        title: "数据保留与归档",
        description: "按保留策略归档并清理历史数据",
        triggerMode: "interval",
        enabled: true,
        intervalSecs: 300,
        cronExpr: null,
        isManual: false,
        displayColorLight: "#c2410c",
        displayColorDark: "#f59e0b",
      },
      {
        taskKey: "raw_compression",
        title: "原始载荷压缩",
        description: "压缩冷数据原始载荷",
        triggerMode: "manual",
        enabled: false,
        intervalSecs: null,
        cronExpr: null,
        isManual: true,
        displayColorLight: "#0f766e",
        displayColorDark: "#2dd4bf",
      },
    ]);
  });
  afterEach(() => {
    act(() => root?.unmount());
    host?.remove();
    host = null;
    root = null;
    apiMocks.fetchManagedTasks.mockReset();
    apiMocks.fetchManagedTaskRuntime.mockReset();
    apiMocks.fetchManagedTaskTimeline.mockReset();
  });

  it("loads the managed task directory", async () => {
    renderPage();
    await flushEffects();
    expect(apiMocks.fetchManagedTasks).toHaveBeenCalledTimes(1);
    expect(apiMocks.fetchManagedTaskRuntime).toHaveBeenCalledTimes(1);
    expect(host?.textContent).toContain("数据保留与归档");
    expect(host?.textContent).toContain("正在执行");
    expect(host?.textContent).toContain("已入队");
    expect(host?.textContent).toContain("等待准入 / 压力延后");
    expect(host?.textContent).toContain("压力冷却让行");
    expect(apiMocks.fetchManagedTaskTimeline).toHaveBeenCalledTimes(1);
    expect(host?.textContent).toContain("raw_compression");
  });

  it("shows manual and scheduled task modes", async () => {
    renderPage();
    await flushEffects();
    expect(host?.textContent).toContain("固定间隔");
    expect(host?.textContent).toContain("手动");
    expect(host?.textContent).toContain("已停用");
  });

  it("combines enabled and trigger filters without hiding the running area", async () => {
    renderPage();
    await flushEffects();
    const user = userEvent.setup();
    const page = within(host as HTMLElement);
    await user.click(page.getByRole("combobox", { name: "启用状态" }));
    await user.click(within(document.body).getByRole("option", { name: "已停用" }));
    expect(host?.textContent).toContain("显示 1 / 2");
    expect(page.getByRole("link", { name: /原始载荷压缩/ })).toBeTruthy();
    expect(page.queryByRole("link", { name: /数据保留与归档/ })).toBeNull();
    expect(host?.textContent).toContain("正在执行");
    expect(host?.textContent).toContain("第 1 位");

    const checkboxes = Array.from(
      host?.querySelectorAll<HTMLInputElement>('input[type="checkbox"]') ?? [],
    );
    for (const checkbox of checkboxes) {
      if (checkbox.checked) await user.click(checkbox);
    }
    const manual = checkboxes.find((checkbox) =>
      checkbox.parentElement?.textContent?.includes("手动"),
    );
    expect(manual).toBeDefined();
    await user.click(manual as HTMLInputElement);
    expect(host?.textContent).toContain("显示 1 / 2");
    expect(host?.textContent).toContain("原始载荷压缩");
  });

  it("renders an explicit unknown state when runtime observation fails", async () => {
    apiMocks.fetchManagedTaskRuntime.mockRejectedValueOnce(new Error("runtime unavailable"));
    renderPage();
    await waitFor(() => expect(host?.textContent).toContain("实时运行观测未知"));
    expect(host?.textContent).toContain("当前是否有任务正在工作未知");
    expect(host?.textContent).toContain("观测未知");
  });
});
