import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { I18nProvider } from "../../i18n";
import type { ModelRoutingLiveResponse } from "../../lib/api";
import { ThemeProvider } from "../../theme";
import { buildModelRoutingGanttData } from "./ModelRoutingGantt";
import { ModelRoutingLivePanel } from "./ModelRoutingLivePanel";

const snapshot: ModelRoutingLiveResponse = {
  generatedAt: "2026-08-16T01:00:00Z",
  groups: [
    {
      model: "gpt-5.5-codex",
      accounts: [
        {
          accountId: 11,
          accountDisplayName: "Ciii",
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
          accountDisplayName: "Ciii2",
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
      accountDisplayName: "Ciii",
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
    },
    {
      id: "event:32",
      kind: "event",
      occurredAt: "2026-08-16T00:59:00Z",
      accountId: 12,
      accountDisplayName: "Ciii2",
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
          onModelChange={vi.fn()}
          onStateChange={vi.fn()}
          onOpenAccount={vi.fn()}
          onOpenInvocation={vi.fn()}
          onRefresh={vi.fn()}
        />
      </ThemeProvider>
    </I18nProvider>,
  );
}

describe("ModelRoutingLivePanel", () => {
  it("renders a model-first lane gantt without account-pool aliases or decision lists", () => {
    const html = renderPanel();

    expect(html).toContain("gpt-5.5-codex");
    expect(html).toContain("gpt-5.4-mini");
    expect(html).toContain('data-testid="model-routing-gantt-gpt-5.5-codex"');
    expect(html).toContain('data-testid="model-routing-gantt-gpt-5.4-mini"');
    expect(html).toContain("请求尝试");
    expect(html).toContain("未知");
    expect(html).toContain("API Key #11");
    expect(html).toContain("API Key #12");
    expect(html).not.toContain("Ciii");
    expect(html).not.toContain("recharts-responsive-container");
    expect(html).not.toContain('data-testid="model-routing-account-');
    expect(html).not.toContain('data-testid="model-routing-record-');
    expect(html).toContain('data-testid="model-routing-live-controls"');
    expect(html).toContain("desktop:justify-end");
    expect(html.indexOf(">刷新</button>")).toBeLessThan(html.indexOf('name="modelRoutingState"'));
    expect(html.indexOf('name="modelRoutingState"')).toBeLessThan(
      html.indexOf('name="modelRoutingModel"'),
    );
    expect(html.indexOf('name="modelRoutingModel"')).toBeLessThan(
      html.indexOf('aria-label="路由时间窗"'),
    );
    const primaryGroup = html.indexOf('data-testid="model-routing-model-group-gpt-5.5-codex"');
    const primaryChart = html.indexOf('data-testid="model-routing-gantt-gpt-5.5-codex"');
    const secondaryGroup = html.indexOf('data-testid="model-routing-model-group-gpt-5.4-mini"');
    const secondaryChart = html.indexOf('data-testid="model-routing-gantt-gpt-5.4-mini"');
    expect(primaryGroup).toBeLessThan(primaryChart);
    expect(primaryChart).toBeLessThan(secondaryGroup);
    expect(secondaryGroup).toBeLessThan(secondaryChart);
    expect(html).toContain("1 条决策");
  });

  it("preserves unknown gaps and renders attempts independently from route-state bands", () => {
    const timeline = buildModelRoutingGanttData({
      model: "gpt-5.5-codex",
      accounts: snapshot.groups[0].accounts,
      records: snapshot.records,
      generatedAt: snapshot.generatedAt,
      window: "1h",
    });

    expect(timeline.lanes).toHaveLength(1);
    expect(timeline.lanes[0].bands.map((band) => band.state)).toEqual(["unknown", "available"]);
    expect(timeline.attempts).toEqual([
      expect.objectContaining({ id: "attempt:31", invokeId: "invoke-31", retryIndex: 1 }),
    ]);
  });

  it("renders one model-routing empty state without inventing a routing attempt", () => {
    const html = renderPanel({ generatedAt: "2026-08-16T01:00:00Z", groups: [], records: [] });

    expect(html).toContain("没有符合筛选条件的 API Key 模型路由状态。");
    expect(html).not.toContain('data-testid="model-routing-gantt-');
  });

  it("keeps the model text visible when the model has an identity icon", () => {
    const html = renderPanel({
      ...snapshot,
      groups: [
        ...snapshot.groups,
        {
          model: "gpt-5.6-terra",
          accounts: [],
        },
      ],
    });

    expect(html).toContain("gpt-5.6-terra");
  });
});
