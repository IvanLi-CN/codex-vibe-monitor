/** @vitest-environment jsdom */

import { waitFor, within } from "@testing-library/dom";
import userEvent from "@testing-library/user-event";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "../../i18n";
import { ThemeProvider } from "../../theme";
import SystemTaskDetailPage from "./SystemTaskDetailPage";

const streamMocks = vi.hoisted(() => ({
  useSubscriptionTopic: vi.fn(),
}));
const timeZoneMock = vi.hoisted(() => ({ value: "Asia/Shanghai" }));

vi.mock("../../hooks/useSubscriptionTopic", () => ({
  useSubscriptionTopic: streamMocks.useSubscriptionTopic,
}));
vi.mock("../../lib/timeZone", () => ({
  getBrowserTimeZone: () => timeZoneMock.value,
}));

vi.mock("../../features/system/TaskWorkloadTrend", async () => {
  const actual = await vi.importActual<typeof import("../../features/system/TaskWorkloadTrend")>(
    "../../features/system/TaskWorkloadTrend",
  );
  return {
    ...actual,
    TaskWorkloadTrend: ({ state, error }: { state: string; error?: string | null }) => (
      <div data-testid="task-workload-state">
        {state}
        {error ? `:${error}` : ""}
      </div>
    ),
  };
});

function makeDetail(
  taskKey: string,
  enabled = true,
  recentRunStatus: "success" | "running" = "success",
): Record<string, unknown> {
  const titles: Record<string, string> = {
    invocation_identity_cleanup: "调用身份清理",
    retention_archive: "数据保留与归档",
  };
  const title = titles[taskKey] ?? taskKey;
  return {
    task: {
      taskKey,
      title,
      description: "维护任务描述",
      triggerMode: "interval",
      enabled,
      intervalSecs: 3600,
      cronExpr: null,
      nextTriggerAt: null,
      nextCatchupAt: null,
      catchupReason: null,
      isManual: false,
      effectiveSchedule: {
        source: "default",
        intervalSecs: 3600,
        cronExpr: null,
        nextTriggerAt: null,
      },
      triggerKinds: ["interval", "manual"],
      effectivePolicy: "默认计划",
      policySource: "worker",
      scheduleEditable: false,
      scheduleCapabilityReason: "计划由 worker 规则管理",
      executionClass: "maintenance",
      measurementCapabilities: {
        pending: { supported: true, unit: "rows", scope: taskKey },
        discovered: { supported: true, unit: "rows", scope: taskKey },
        processed: { supported: true, unit: "rows", scope: taskKey },
      },
    },
    progress: {
      phase: enabled ? "idle" : "paused",
      freshness: "fresh",
      sourceScope: "对话身份和小时前缀",
      catchupState: enabled ? "idle" : "disabled",
      waitReason: null,
      checkpoint: "checkpoint-1",
    },
    recentRuns: [
      {
        id: 1,
        triggerKind: "manual",
        startedAt: "2026-10-04T11:59:00.000Z",
        executionUid: "test-execution-1",
        actualStartedAt: "2026-10-04T12:00:00.000Z",
        actualFinishedAt: recentRunStatus === "success" ? "2026-10-04T12:00:01.000Z" : null,
        executionDurationMs: recentRunStatus === "success" ? 1000 : null,
        schedulerWaitMs: recentRunStatus === "success" ? 60_000 : null,
        finishedAt: recentRunStatus === "success" ? "2026-10-04T12:00:01.000Z" : null,
        durationMs: recentRunStatus === "success" ? 61_000 : null,
        status: recentRunStatus,
        summary: "本轮维护结果",
        details: {
          ownershipVersion: 1,
          dryRun: false,
          resourceWaitMs: 30_000,
          fileResourceWaitMs: 20_000,
          conversationIdentitiesChecked: 32,
          conversationIdentitiesReleased: 4,
          hourPrefixesChecked: 32,
          hourPrefixesReleased: 3,
        },
      },
    ],
    retentionBacklogTrend: [],
    workloadTrend: null,
  };
}

let host: HTMLDivElement | null = null;
let root: Root | null = null;
let currentDetail: Record<string, unknown>;
let failDetailLoad = false;
let failManualRun = false;
let fetchMock: ReturnType<typeof vi.fn>;
let subscribedRuntime: { activeRuns: Array<Record<string, unknown>> } | null = null;
let subscribedDetail: Record<string, unknown> | null = null;

function taskDetailTree(taskKey: string) {
  return (
    <MemoryRouter initialEntries={[`/system/tasks/${taskKey}`]}>
      <ThemeProvider>
        <I18nProvider>
          <Routes>
            <Route path="/system/tasks/:taskKey" element={<SystemTaskDetailPage />} />
          </Routes>
        </I18nProvider>
      </ThemeProvider>
    </MemoryRouter>
  );
}

function renderPage(taskKey: string) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(taskDetailTree(taskKey));
  });
}

function rerenderPage(taskKey: string) {
  act(() => root?.render(taskDetailTree(taskKey)));
}

async function waitForText(text: string | RegExp) {
  await waitFor(() => {
    const bodyText = host?.textContent ?? "";
    if (typeof text === "string") {
      expect(bodyText).toContain(text);
    } else {
      expect(bodyText).toMatch(text);
    }
  });
}

describe("SystemTaskDetailPage", () => {
  beforeEach(() => {
    timeZoneMock.value = "Asia/Shanghai";
    currentDetail = makeDetail("invocation_identity_cleanup");
    failDetailLoad = false;
    failManualRun = false;
    subscribedRuntime = null;
    subscribedDetail = null;
    streamMocks.useSubscriptionTopic.mockImplementation((descriptor: { topic: string }) => ({
      data:
        descriptor.topic === "system.managed-tasks.runtime" ? subscribedRuntime : subscribedDetail,
      lastReceivedAt: null,
      error: null,
      isLoading: false,
      refresh: vi.fn(),
    }));
    fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input), "http://test.invalid");
      if (url.pathname === "/api/system/observability") {
        return new Response(JSON.stringify({}), { status: 200 });
      }
      if (url.pathname.endsWith("/run") && init?.method === "POST") {
        if (failManualRun) {
          return new Response(JSON.stringify({ error: "task already has an active run" }), {
            status: 409,
          });
        }
        const task = currentDetail.task as Record<string, unknown>;
        currentDetail = makeDetail(String(task.taskKey), task.enabled === true, "running");
        return new Response(JSON.stringify(currentDetail), { status: 200 });
      }
      if (init?.method === "PATCH") {
        const payload = JSON.parse(String(init.body ?? "{}")) as { enabled?: boolean };
        const task = currentDetail.task as Record<string, unknown>;
        currentDetail = makeDetail(
          String(task.taskKey),
          payload.enabled ?? task.enabled === true,
          "success",
        );
        return new Response(JSON.stringify(currentDetail), { status: 200 });
      }
      if (url.pathname.startsWith("/api/system/managed-tasks/")) {
        if (failDetailLoad) {
          return new Response(JSON.stringify({ error: "managed task unavailable" }), {
            status: 503,
          });
        }
        return new Response(JSON.stringify(currentDetail), { status: 200 });
      }
      throw new Error(`Unexpected request: ${url.pathname}`);
    });
    vi.stubGlobal("fetch", fetchMock);
  });

  afterEach(() => {
    act(() => root?.unmount());
    host?.remove();
    host = null;
    root = null;
    streamMocks.useSubscriptionTopic.mockReset();
    vi.unstubAllGlobals();
  });

  it("renders measured units and the paused-manual execution help", async () => {
    renderPage("invocation_identity_cleanup");
    await waitForText("调用身份清理");

    expect(host?.textContent).toContain("默认计划");
    expect(host?.textContent).toContain("3600s");
    expect(host?.textContent).toContain("对话身份 32 个已检查 / 4 个已释放");
    expect(host?.textContent).toContain("暂停后仍可立即运行");
  });

  it("keeps retention maintenance links visible", async () => {
    currentDetail = makeDetail("retention_archive");
    renderPage("retention_archive");
    await waitForText("数据保留与归档");

    const links = within(host as HTMLElement).getByRole("navigation", { name: "关联维护任务" });
    expect(
      links.querySelector('a[href="/system/tasks/prompt_cache_materialization"]'),
    ).not.toBeNull();
    expect(
      links.querySelector('a[href="/system/tasks/invocation_identity_cleanup"]'),
    ).not.toBeNull();
    expect(links.querySelector('a[href="/system/tasks/raw_orphan_sweep"]')).not.toBeNull();
  });

  it("separates trigger, execution, and scheduler timing and copies the full identity", async () => {
    const user = userEvent.setup();
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });
    currentDetail = makeDetail("retention_archive");
    renderPage("retention_archive");
    await waitForText("数据保留与归档");

    expect(host?.textContent).toContain("触发时间");
    expect(host?.textContent).toContain("实际开始");
    expect(host?.textContent).toContain("实际结束");
    expect(host?.textContent).toContain("执行用时");
    expect(host?.textContent).toContain("调度等待");
    expect(host?.textContent).toContain("调度侧资源准入等待");
    expect(host?.textContent).toContain("文件资源等待");
    expect(host?.textContent).toContain("test-execution-1");
    expect(host?.textContent).toContain("1.0 s");
    expect(host?.textContent).toContain("60.0 s");
    expect(host?.textContent).toContain("30.0 s");
    expect(host?.textContent).toContain("20.0 s");
    expect(host?.textContent).not.toContain("61.0 s");

    await user.click(within(host as HTMLElement).getByRole("button", { name: "复制执行身份" }));
    await waitFor(() => expect(host?.textContent).toContain("已复制"));
    expect(writeText).toHaveBeenCalledWith("test-execution-1");
  });

  it("uses the browser timezone for absolute task timestamps", async () => {
    currentDetail = makeDetail("invocation_identity_cleanup");
    renderPage("invocation_identity_cleanup");
    await waitForText("调用身份清理");
    expect(host?.textContent).toContain("19:59:00");

    timeZoneMock.value = "UTC";
    rerenderPage("invocation_identity_cleanup");
    expect(host?.textContent).toContain("11:59:00");

    timeZoneMock.value = "America/Los_Angeles";
    rerenderPage("invocation_identity_cleanup");
    expect(host?.textContent).toContain("04:59:00");
    expect(host?.textContent).toContain("2026/10/04");
  });

  it("keeps legacy run duration unknown when actual execution timing is absent", async () => {
    const legacyDetail = makeDetail("invocation_identity_cleanup") as {
      recentRuns: Array<Record<string, unknown>>;
    };
    legacyDetail.recentRuns = [
      {
        id: 99,
        startedAt: "2026-10-04T12:00:00.000Z",
        finishedAt: "2026-10-04T12:31:00.000Z",
        durationMs: 1_860_000,
        status: "success",
      },
    ];
    currentDetail = legacyDetail;
    renderPage("invocation_identity_cleanup");
    await waitForText("调用身份清理");

    expect(host?.textContent).toContain("运行记录 ID：99");
    expect(host?.textContent).toContain("执行用时");
    expect(host?.textContent).not.toContain("1860.0 s");
  });

  it("applies runtime and detail subscription updates", async () => {
    renderPage("invocation_identity_cleanup");
    await waitForText("调用身份清理");

    subscribedDetail = makeDetail("invocation_identity_cleanup", false);
    subscribedRuntime = {
      activeRuns: [
        {
          executionId: 7,
          executionUid: "subscription-run",
          taskKey: "invocation_identity_cleanup",
          title: "调用身份清理",
          triggerKind: "manual",
          phase: "processing",
          executionClass: "maintenance",
          startedAt: "2026-10-04T12:00:00.000Z",
          elapsedMs: 1200,
          activeChildTaskKey: null,
          activeChildTitle: "释放对话身份",
        },
      ],
    };
    rerenderPage("invocation_identity_cleanup");

    await waitForText("恢复自动触发");
    await waitForText("当前子任务：释放对话身份");
    expect(
      (within(host as HTMLElement).getByRole("button", { name: "运行中" }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);

    subscribedDetail = makeDetail("invocation_identity_cleanup", true);
    subscribedRuntime = { activeRuns: [] };
    rerenderPage("invocation_identity_cleanup");

    await waitForText("暂停自动触发");
    expect(
      (within(host as HTMLElement).getByRole("button", { name: "立即运行" }) as HTMLButtonElement)
        .disabled,
    ).toBe(false);
  });

  it("keeps a paused task manually runnable and reflects the active run", async () => {
    currentDetail = makeDetail("retention_archive");
    const user = userEvent.setup();
    renderPage("retention_archive");
    await waitForText("数据保留与归档");

    await user.click(within(host as HTMLElement).getByRole("button", { name: "暂停自动触发" }));
    await waitForText("恢复自动触发");
    const runButton = within(host as HTMLElement).getByRole("button", { name: "立即运行" });
    expect((runButton as HTMLButtonElement).disabled).toBe(false);

    await user.click(runButton);
    await waitFor(() => {
      const activeRunButton = within(host as HTMLElement).getByRole("button", { name: "运行中" });
      expect((activeRunButton as HTMLButtonElement).disabled).toBe(true);
    });
    expect(host?.textContent).toContain("暂停后仍可立即运行");

    const patchCall = fetchMock.mock.calls.find(
      ([input, init]) =>
        String(input).includes("/api/system/managed-tasks/retention_archive") &&
        init?.method === "PATCH",
    );
    expect(patchCall?.[1]?.body).toBe('{"enabled":false}');
    expect(
      fetchMock.mock.calls.some(
        ([input, init]) =>
          String(input).endsWith("/api/system/managed-tasks/retention_archive/run") &&
          init?.method === "POST",
      ),
    ).toBe(true);
  });

  it("renders API errors from a rejected manual run", async () => {
    failManualRun = true;
    const user = userEvent.setup();
    renderPage("invocation_identity_cleanup");
    await waitForText("调用身份清理");

    await user.click(within(host as HTMLElement).getByRole("button", { name: "立即运行" }));
    await waitFor(() =>
      expect(within(host as HTMLElement).getByRole("status").textContent).toContain(
        'Request failed: 409 {"error":"task already has an active run"}',
      ),
    );
    expect(
      (within(host as HTMLElement).getByRole("button", { name: "立即运行" }) as HTMLButtonElement)
        .disabled,
    ).toBe(false);
  });

  it("renders the loading fallback error when the detail request fails", async () => {
    failDetailLoad = true;
    renderPage("invocation_identity_cleanup");

    await waitFor(() =>
      expect(within(host as HTMLElement).getByRole("status").textContent).toContain(
        'Request failed: 503 {"error":"managed task unavailable"}',
      ),
    );
  });
});
