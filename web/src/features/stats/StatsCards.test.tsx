/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { StatsCards } from "./StatsCards";

vi.mock("../../i18n", () => ({
  useTranslation: () => ({
    locale: "en",
    t: (key: string, values?: { error?: string }) => {
      const map: Record<string, string> = {
        "stats.cards.loadError": `Load error: ${values?.error ?? "unknown"}`,
        "stats.cards.recoveryPending": "Summary recovery pending",
        "stats.cards.recoveryFailed": "Summary recovery failed",
        "stats.cards.totalCalls": "Total calls",
        "stats.cards.success": "Success",
        "stats.cards.failures": "Failures",
        "stats.cards.totalCost": "Total cost",
        "stats.cards.totalTokens": "Total tokens",
      };
      return map[key] ?? key;
    },
  }),
}));

let host: HTMLDivElement | null = null;
let root: Root | null = null;

function render(ui: React.ReactNode) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(ui);
  });
}

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  root = null;
  host = null;
});

describe("StatsCards", () => {
  it("keeps recovery errors in the metric layout and distinguishes pending from failed", () => {
    render(
      <StatsCards
        stats={null}
        loading={false}
        error="Request failed"
        errorCode="summary_recovery_pending"
      />,
    );

    expect(host?.querySelector('[data-testid="stats-cards-grid"]')).not.toBeNull();
    expect(host?.querySelector('[data-testid="stats-cards-grid"]')?.getAttribute("inert")).toBe("");
    expect(
      host?.querySelector('[data-testid="stats-cards-state"]')?.getAttribute("aria-busy"),
    ).toBe("true");
    expect(host?.querySelector('[data-testid="stats-cards-error-overlay"]')?.textContent).toContain(
      "Summary recovery pending",
    );
    expect(host?.querySelectorAll(".metric-cell")).toHaveLength(5);
    expect(host?.querySelectorAll('[data-testid$="-loading"]')).toHaveLength(5);

    act(() => {
      root?.render(
        <StatsCards
          stats={null}
          loading={false}
          error="Request failed"
          errorCode="summary_recovery_failed"
        />,
      );
    });

    expect(host?.querySelector('[data-testid="stats-cards-error-overlay"]')?.textContent).toContain(
      "Summary recovery failed",
    );
    expect(
      host?.querySelector('[data-testid="stats-cards-state"]')?.getAttribute("aria-busy"),
    ).toBe(null);
  });
});
