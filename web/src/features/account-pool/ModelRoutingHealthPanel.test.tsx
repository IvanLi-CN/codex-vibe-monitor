import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { I18nProvider } from "../../i18n";
import { ModelRoutingHealthPanel } from "./ModelRoutingHealthPanel";

describe("ModelRoutingHealthPanel", () => {
  it("keeps model health compact until the operator expands a model", () => {
    const html = renderToStaticMarkup(
      <I18nProvider>
        <ModelRoutingHealthPanel
          accountId={21}
          states={[
            {
              model: "gpt-5.5-codex",
              state: "cooling_down",
              priority: "excluded",
              failureCount: 3,
              changedAt: "2026-08-16T00:00:00Z",
              lastSeenAt: "2026-08-16T01:00:00Z",
              cooldownUntil: "2026-08-16T01:15:00Z",
              cacheConcurrencyLimit: 1,
              cacheLastHitRatePercent: 3,
              probeRequired: true,
            },
            {
              model: "future-route-state",
              state: "probing",
              priority: "normal",
              failureCount: 0,
              changedAt: "2026-08-16T00:00:00Z",
              lastSeenAt: "2026-08-16T00:00:00Z",
            },
          ]}
          writesEnabled
          onReset={vi.fn()}
        />
      </I18nProvider>,
    );

    expect(html).toContain('data-testid="upstream-account-model-routing-panel"');
    expect(html).toContain("gpt-5.5-codex");
    expect(html).toContain("未知结果");
    expect(html).toContain('aria-expanded="false"');
    expect(html).toContain('aria-label="恢复可用: gpt-5.5-codex"');
    expect(html).not.toContain("加载更多");
  });

  it("uses the shared error alert preset for routing load failures", () => {
    const html = renderToStaticMarkup(
      <I18nProvider>
        <ModelRoutingHealthPanel
          accountId={21}
          states={[]}
          error="模型路由状态刷新失败"
          writesEnabled
          onReset={vi.fn()}
        />
      </I18nProvider>,
    );

    expect(html).toContain('data-testid="model-routing-error"');
    expect(html).toContain("border-error/45");
    expect(html).toContain("bg-error/15");
    expect(html).toContain("tone-ink-error");
  });
});
