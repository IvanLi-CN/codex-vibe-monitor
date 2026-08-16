import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { I18nProvider } from "../../i18n";
import type { ModelRoutingLiveResponse } from "../../lib/api";
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
          accountGroupName: "primary",
          model: "gpt-5.5-codex",
          state: "available",
          priority: "normal",
          failureCount: 0,
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
          accountGroupName: "fallback",
          model: "gpt-5.4-mini",
          state: "cooling_down",
          priority: "excluded",
          failureCount: 2,
          lastSeenAt: "2026-08-16T00:59:00Z",
        },
      ],
    },
  ],
  records: [
    {
      id: "attempt:31",
      kind: "attempt",
      occurredAt: "2026-08-16T01:00:00Z",
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
    </I18nProvider>,
  );
}

describe("ModelRoutingLivePanel", () => {
  it("keeps account states and routing attempts within their model group", () => {
    const html = renderPanel();

    expect(html).toContain("gpt-5.5-codex");
    expect(html).toContain("gpt-5.4-mini");
    expect(html).toContain("Ciii");
    expect(html).toContain("Ciii2");
    expect(html).toContain('data-testid="model-routing-account-11-gpt-5.5-codex"');
    expect(html).toContain('data-testid="model-routing-record-attempt:31"');
    expect(html).toContain('data-testid="model-routing-model-records-gpt-5.5-codex"');
    expect(html).toContain('data-testid="model-routing-model-records-gpt-5.4-mini"');
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
    const primaryRecord = html.indexOf('data-testid="model-routing-record-attempt:31"');
    const secondaryGroup = html.indexOf('data-testid="model-routing-model-group-gpt-5.4-mini"');
    const secondaryRecord = html.indexOf('data-testid="model-routing-record-event:32"');
    expect(primaryGroup).toBeLessThan(primaryRecord);
    expect(primaryRecord).toBeLessThan(secondaryGroup);
    expect(secondaryGroup).toBeLessThan(secondaryRecord);
    expect(html).toContain("1 条决策");
  });

  it("renders one model-routing empty state without inventing a routing attempt", () => {
    const html = renderPanel({ generatedAt: "2026-08-16T01:00:00Z", groups: [], records: [] });

    expect(html).toContain("没有符合筛选条件的 API Key 模型路由状态。");
    expect(html).not.toContain('data-testid="model-routing-model-records-');
  });
});
