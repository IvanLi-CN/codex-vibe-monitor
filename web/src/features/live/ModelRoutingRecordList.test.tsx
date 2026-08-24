/** @vitest-environment jsdom */
import { act, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "../../i18n";
import type { ModelRoutingTimelineRecord } from "../../lib/api";
import { ModelRoutingRecordList } from "./ModelRoutingRecordList";

const records: ModelRoutingTimelineRecord[] = [
  {
    id: "attempt:gpt-5.5",
    kind: "attempt",
    occurredAt: "2026-08-16T03:59:20.000Z",
    accountId: 11,
    accountDisplayName: "Aster",
    model: "gpt-5.5",
    invokeId: "invoke-11",
    sameAccountRetryIndex: 1,
    status: "success",
    httpStatus: 200,
    totalLatencyMs: 821,
    reasonCode: "probe_passed",
    routingSource: "pool",
    modelRouteStateBefore: "cooling_down",
    modelRouteStateAfter: "available",
    modelRoutePriorityBefore: "excluded",
    modelRoutePriorityAfter: "normal",
    routingSelectionAudit: {
      selectedAccountId: 11,
      selectedAccountName: "Aster",
      eligibleCandidateCount: 2,
      winnerReasonCode: "lowest_effective_load",
      comparedAccountId: 12,
      comparedAccountName: "Borealis",
      excludedCandidates: [
        {
          accountId: 13,
          accountName: "Cedar",
          reasonCode: "cooling_down",
        },
      ],
    },
  },
  {
    id: "event:gpt-5.5",
    kind: "event",
    occurredAt: "2026-08-16T04:00:00.000Z",
    accountId: 12,
    accountDisplayName: "Borealis",
    model: "gpt-5.5",
    action: "model_route_cooldown",
    reasonCode: "upstream_http_5xx",
    modelRouteStateBefore: "degraded",
    modelRouteStateAfter: "cooling_down",
  },
  {
    id: "reset:gpt-5.5",
    kind: "event",
    occurredAt: "2026-08-16T04:00:05.000Z",
    accountId: 12,
    model: "gpt-5.5",
    action: "model_route_reset",
    reasonCode: "model_route",
  },
  {
    id: "attempt:other-model",
    kind: "attempt",
    occurredAt: "2026-08-16T04:00:10.000Z",
    accountId: 21,
    accountDisplayName: "Lumen",
    model: "gpt-5.4-mini",
    status: "success",
  },
];

let host: HTMLDivElement | null = null;
let root: Root | null = null;

beforeAll(() => {
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
    configurable: true,
    writable: true,
    value: true,
  });
});

afterEach(() => {
  act(() => root?.unmount());
  host?.remove();
  host = null;
  root = null;
  vi.clearAllMocks();
});

function render(ui: ReactNode) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => root?.render(ui));
}

describe("ModelRoutingRecordList", () => {
  it("shows every returned record for the selected model and expands decision evidence", () => {
    const onOpenAccount = vi.fn();
    const onOpenInvocation = vi.fn();
    render(
      <I18nProvider>
        <ModelRoutingRecordList
          model="gpt-5.5"
          records={records}
          onOpenAccount={onOpenAccount}
          onOpenInvocation={onOpenInvocation}
        />
      </I18nProvider>,
    );

    expect(host?.querySelectorAll('[data-testid^="model-routing-record-"]')).toHaveLength(3);
    expect(host?.textContent).toContain("重试 1");
    expect(host?.textContent).toContain("状态事件");
    expect(host?.textContent).toContain("模型路由已重置");
    expect(host?.querySelector("h3")?.textContent).toBe("路由记录");
    expect(
      host?.querySelector('[data-testid="model-routing-model-records-gpt-5.5"]')?.className,
    ).not.toContain("h-full");
    expect(
      host?.querySelector('[data-testid="model-routing-model-records-gpt-5.5"] > div:last-child')
        ?.className,
    ).not.toContain("overflow-y-auto");
    expect(host?.textContent).not.toContain("gpt-5.5 路由决策");
    expect(host?.textContent).not.toContain("gpt-5.4-mini");
    expect(host?.textContent).toContain("Aster");

    const attempt = host?.querySelector('[data-testid="model-routing-record-attempt:gpt-5.5"]');
    const toggle = attempt?.querySelector('button[aria-label="展开决策详情"]');
    if (!(toggle instanceof HTMLButtonElement)) throw new Error("missing record toggle");
    act(() => toggle.click());

    expect(attempt?.textContent).toContain("有效负载最低");
    expect(attempt?.textContent).toContain("与 Borealis 比较");
    expect(attempt?.textContent).toContain("Cedar (冷却中)");
    expect(attempt?.textContent).toContain("冷却中 → 可用");
    expect(attempt?.textContent).toContain("排除 → 正常");

    const account = Array.from(attempt?.querySelectorAll("button") ?? []).find(
      (button) => button.textContent?.trim() === "Aster",
    );
    const invocation = attempt?.querySelector('button[aria-label="打开调用详情"]');
    if (!(account instanceof HTMLButtonElement) || !(invocation instanceof HTMLButtonElement)) {
      throw new Error("missing route drilldown controls");
    }
    act(() => {
      account.click();
      invocation.click();
    });
    expect(onOpenAccount).toHaveBeenCalledWith(11, "gpt-5.5");
    expect(onOpenInvocation).toHaveBeenCalledWith("invoke-11");
  });
});
