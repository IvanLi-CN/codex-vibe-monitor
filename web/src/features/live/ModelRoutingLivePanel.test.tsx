import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { I18nProvider } from "../../i18n";
import type { ModelRoutingLiveResponse } from "../../lib/api";
import { ThemeProvider } from "../../theme";
import {
  availableBandOpacity,
  buildFrappeRoutingTasks,
  buildFrappeSystemRoutingTasks,
  buildModelRoutingGanttData,
} from "./ModelRoutingGantt";
import { ModelRoutingLivePanel } from "./ModelRoutingLivePanel";

const primaryAccountName = "Aster";
const secondaryAccountName = "Borealis";

const snapshot: ModelRoutingLiveResponse = {
  generatedAt: "2026-08-16T01:00:00Z",
  groups: [
    {
      model: "gpt-5.5-codex",
      accounts: [
        {
          accountId: 11,
          accountDisplayName: primaryAccountName,
          model: "gpt-5.5-codex",
          state: "available",
          priority: "normal",
          failureCount: 0,
          changedAt: "2026-08-16T00:30:00Z",
          lastSeenAt: "2026-08-16T01:00:00Z",
          cacheConcurrencyLimit: 1,
          probeRequired: true,
        },
      ],
    },
    {
      model: "gpt-5.4-mini",
      accounts: [
        {
          accountId: 12,
          accountDisplayName: secondaryAccountName,
          model: "gpt-5.4-mini",
          state: "cooling_down",
          priority: "excluded",
          failureCount: 2,
          changedAt: "2026-08-16T00:59:00Z",
          lastSeenAt: "2026-08-16T00:59:00Z",
        },
      ],
    },
  ],
  records: [
    {
      id: "attempt:31",
      kind: "attempt",
      occurredAt: "2026-08-16T00:30:00Z",
      accountId: 11,
      accountDisplayName: primaryAccountName,
      model: "gpt-5.5-codex",
      attemptId: "attempt-public-31",
      invokeId: "invoke-31",
      attemptIndex: 2,
      sameAccountRetryIndex: 1,
      status: "success",
      httpStatus: 200,
      totalLatencyMs: 812,
      reasonCode: "recovery_after_cooldown",
      modelRouteStateBefore: "cooling_down",
      modelRouteStateAfter: "available",
      modelRoutePriorityBefore: "excluded",
      modelRoutePriorityAfter: "normal",
    },
    {
      id: "event:32",
      kind: "event",
      occurredAt: "2026-08-16T00:59:00Z",
      accountId: 12,
      accountDisplayName: secondaryAccountName,
      model: "gpt-5.4-mini",
      status: "cooling_down",
      action: "model_route_cooldown",
      reasonCode: "upstream_http_5xx",
      modelRouteStateBefore: "degraded",
      modelRouteStateAfter: "cooling_down",
    },
  ],
};

function renderPanel(data: ModelRoutingLiveResponse | null = snapshot) {
  return renderToStaticMarkup(
    <I18nProvider>
      <ThemeProvider>
        <ModelRoutingLivePanel
          data={data}
          isLoading={false}
          error={null}
          window="1h"
          onWindowChange={vi.fn()}
          onOpenAccount={vi.fn()}
          onOpenInvocation={vi.fn()}
          onRefresh={vi.fn()}
        />
      </ThemeProvider>
    </I18nProvider>,
  );
}

describe("ModelRoutingLivePanel", () => {
  it("maps available-band color intensity to relative real-request allocation", () => {
    expect(availableBandOpacity(0, 10)).toBeCloseTo(0.3);
    expect(availableBandOpacity(5, 10)).toBeCloseTo(0.65);
    expect(availableBandOpacity(10, 10)).toBeCloseTo(1);
    expect(availableBandOpacity(10, 0)).toBeCloseTo(0.56);
  });

  it("renders model-first SVG gantt hosts without the removed HTML grid", () => {
    const html = renderPanel();

    expect(html).toContain('data-testid="model-routing-gantt"');
    expect(html).toContain('data-testid="model-routing-gantt-chart-system"');
    expect(html).not.toContain('data-testid="model-routing-gantt-grid"');
    expect(html.match(/data-testid="model-routing-gantt-legend"/g)).toHaveLength(1);
    expect(html).toContain("恢复尝试");
    expect(html).toContain("未知");
    expect(html).not.toContain(primaryAccountName);
    expect(html).not.toContain("recharts-responsive-container");
    expect(html).not.toContain('data-testid="model-routing-account-');
    expect(html).not.toContain('data-testid="model-routing-record-');
    expect(html).toContain('data-testid="model-routing-live-controls"');
    expect(html).toContain("desktop:justify-end");
    expect(html).not.toContain('name="modelRoutingState"');
    expect(html).not.toContain('name="modelRoutingModel"');
    expect(html.indexOf(">刷新</button>")).toBeLessThan(html.indexOf('aria-label="路由时间窗"'));
    const tasks = buildFrappeSystemRoutingTasks(
      snapshot.groups.map((group) => ({
        model: group.model,
        accountCount: group.accounts.length,
        recordCount: snapshot.records.filter((record) => record.model === group.model).length,
        timeline: buildModelRoutingGanttData({
          model: group.model,
          accounts: group.accounts,
          records: snapshot.records,
          generatedAt: snapshot.generatedAt,
          window: "1h",
        }),
      })),
    );
    expect(tasks.map((task) => task.name)).toEqual([
      "gpt-5.5-codex",
      primaryAccountName,
      "gpt-5.4-mini",
      secondaryAccountName,
    ]);

    const expandedTasks = buildFrappeSystemRoutingTasks(
      snapshot.groups.map((group) => ({
        model: group.model,
        accountCount: group.accounts.length,
        recordCount: snapshot.records.filter((record) => record.model === group.model).length,
        timeline: buildModelRoutingGanttData({
          model: group.model,
          accounts: group.accounts,
          records: snapshot.records,
          generatedAt: snapshot.generatedAt,
          window: "1h",
        }),
      })),
      undefined,
      "gpt-5.5-codex",
    );
    const selectedLaneIndex = expandedTasks.findIndex(
      (task) => task.id === "route-gpt-5x2ex5-codex78x-11",
    );
    const nextModelIndex = expandedTasks.findIndex((task) => task.id === "model-gpt-5x2ex4-mini");
    const detailTasks = expandedTasks.slice(selectedLaneIndex + 1, nextModelIndex);
    expect(detailTasks.length).toBeGreaterThan(0);
    expect(detailTasks.every((task) => task.kind === "detail")).toBe(true);
  });

  it("preserves all calls for allocation but marks only controlled recovery attempts", () => {
    const ordinaryAttempt = {
      ...snapshot.records[0],
      id: "attempt:ordinary",
      occurredAt: "2026-08-16T00:45:00Z",
      invokeId: "invoke-ordinary",
      reasonCode: "selected_eligible_route",
      modelRouteStateBefore: "available",
      modelRouteStateAfter: "available",
    };
    const timeline = buildModelRoutingGanttData({
      model: "gpt-5.5-codex",
      accounts: snapshot.groups[0].accounts,
      records: [...snapshot.records, ordinaryAttempt],
      generatedAt: snapshot.generatedAt,
      window: "1h",
    });

    expect(timeline.lanes).toHaveLength(1);
    expect(timeline.lanes[0].bands.map((band) => band.state)).toEqual([
      "cooling_down",
      "available",
    ]);
    expect(timeline.lanes[0].bands.map((band) => band.priority)).toEqual(["excluded", "normal"]);
    expect(timeline.attempts).toEqual([
      expect.objectContaining({ id: "attempt:31" }),
      expect.objectContaining({ id: "attempt:ordinary" }),
    ]);
    expect(timeline.recoveryAttempts).toEqual([
      expect.objectContaining({
        id: "attempt:31",
        invokeId: "invoke-31",
        retryIndex: 1,
      }),
    ]);
    expect(buildFrappeRoutingTasks(timeline)).toEqual([
      expect.objectContaining({
        id: "route-gpt-5x2ex5-codex78x-11",
        name: primaryAccountName,
        accountId: 11,
        model: "gpt-5.5-codex",
        custom_class: "model-routing-task",
      }),
    ]);
  });

  it("reconstructs model-route priority as time-based task segments", () => {
    const base = snapshot.groups[0].accounts[0];
    const timeline = buildModelRoutingGanttData({
      model: base.model,
      accounts: [
        {
          ...base,
          state: "cooling_down",
          priority: "excluded",
          changedAt: "2026-08-16T00:40:00Z",
        },
      ],
      records: [
        {
          ...snapshot.records[0],
          id: "event:demoted",
          occurredAt: "2026-08-16T00:20:00Z",
          modelRouteStateBefore: "available",
          modelRouteStateAfter: "degraded",
          modelRoutePriorityBefore: "normal",
          modelRoutePriorityAfter: "demoted",
        },
        {
          ...snapshot.records[0],
          id: "event:excluded",
          occurredAt: "2026-08-16T00:40:00Z",
          modelRouteStateBefore: "degraded",
          modelRouteStateAfter: "cooling_down",
          modelRoutePriorityBefore: "demoted",
          modelRoutePriorityAfter: "excluded",
        },
      ],
      generatedAt: snapshot.generatedAt,
      window: "1h",
    });

    expect(timeline.lanes[0].bands.map((band) => band.priority)).toEqual([
      "normal",
      "demoted",
      "excluded",
    ]);
    expect(timeline.lanes[0].bands.map((band) => band.startMs)).toEqual([
      Date.parse("2026-08-16T00:00:00Z"),
      Date.parse("2026-08-16T00:20:00Z"),
      Date.parse("2026-08-16T00:40:00Z"),
    ]);
  });

  it("renders one model-routing empty state without inventing a routing attempt", () => {
    const html = renderPanel({
      generatedAt: "2026-08-16T01:00:00Z",
      groups: [],
      records: [],
    });

    expect(html).toContain("没有符合筛选条件的 API Key 模型路由状态。");
    expect(html).not.toContain('data-testid="model-routing-gantt"');
  });

  it("keeps a model group task when the model has no account lanes", () => {
    const timeline = buildModelRoutingGanttData({
      model: "gpt-5.6-terra",
      accounts: [],
      records: [],
      generatedAt: snapshot.generatedAt,
      window: "1h",
    });
    const tasks = buildFrappeSystemRoutingTasks([
      {
        model: "gpt-5.6-terra",
        accountCount: 0,
        recordCount: 0,
        timeline,
      },
    ]);

    expect(tasks).toEqual([
      expect.objectContaining({
        name: "gpt-5.6-terra",
        kind: "model",
        custom_class: "model-routing-model-task",
      }),
    ]);
  });

  it("keeps task IDs distinct for models whose punctuation would otherwise collide", () => {
    const groups = ["foo/bar", "foo-bar"].map((model, index) => ({
      model,
      accountCount: 1,
      recordCount: 0,
      timeline: buildModelRoutingGanttData({
        model,
        accounts: [
          {
            accountId: index + 1,
            accountDisplayName: index === 0 ? primaryAccountName : secondaryAccountName,
            model,
            state: "available" as const,
            priority: "normal" as const,
            failureCount: 0,
            lastSeenAt: snapshot.generatedAt,
          },
        ],
        records: [],
        generatedAt: snapshot.generatedAt,
        window: "1h",
      }),
    }));

    const tasks = buildFrappeSystemRoutingTasks(groups);
    expect(new Set(tasks.map((task) => task.id)).size).toBe(tasks.length);
  });
});
