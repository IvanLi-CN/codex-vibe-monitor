/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "../../i18n";
import SystemTasksPage from "./SystemTasksPage";

const apiMocks = vi.hoisted(() => ({ fetchManagedTasks: vi.fn() }));
vi.mock("../../lib/api", async () => ({
  ...(await vi.importActual<typeof import("../../lib/api")>("../../lib/api")),
  fetchManagedTasks: apiMocks.fetchManagedTasks,
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
      },
    ]);
  });
  afterEach(() => {
    act(() => root?.unmount());
    host?.remove();
    host = null;
    root = null;
    apiMocks.fetchManagedTasks.mockReset();
  });

  it("loads the managed task directory", async () => {
    renderPage();
    await flushEffects();
    expect(apiMocks.fetchManagedTasks).toHaveBeenCalledTimes(1);
    expect(host?.textContent).toContain("数据保留与归档");
    expect(host?.textContent).toContain("raw_compression");
  });

  it("shows manual and scheduled task modes", async () => {
    renderPage();
    await flushEffects();
    expect(host?.textContent).toContain("固定间隔");
    expect(host?.textContent).toContain("手动");
    expect(host?.textContent).toContain("已停用");
  });
});
