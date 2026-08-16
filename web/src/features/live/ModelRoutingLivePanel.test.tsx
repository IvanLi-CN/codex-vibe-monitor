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
  it("renders model-first account state and each routing attempt", () => {
    const html = renderPanel();

    expect(html).toContain("gpt-5.5-codex");
    expect(html).toContain("Ciii");
    expect(html).toContain('data-testid="model-routing-account-11-gpt-5.5-codex"');
    expect(html).toContain('data-testid="model-routing-record-attempt:31"');
    expect(html).toContain("1/100");
  });

  it("renders the empty state without inventing a routing attempt", () => {
    const html = renderPanel({ generatedAt: "2026-08-16T01:00:00Z", groups: [], records: [] });

    expect(html).toContain("没有符合筛选条件的 API Key 模型路由状态。");
    expect(html).toContain("当前时间窗内没有路由尝试或状态事件。");
  });
});
