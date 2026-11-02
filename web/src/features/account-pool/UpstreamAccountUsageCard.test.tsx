import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { UpstreamAccountHistoryPoint } from "../../lib/api";
import { ReserveQuotaCard, UpstreamAccountUsageCard } from "./UpstreamAccountUsageCard";

const emptyLabel = "No usage samples yet";
const history: UpstreamAccountHistoryPoint[] = [
  {
    capturedAt: "2026-03-10T02:00:00.000Z",
    primaryUsedPercent: 24,
    secondaryUsedPercent: 10,
  },
];

function renderCard(overrides: Partial<Parameters<typeof UpstreamAccountUsageCard>[0]> = {}) {
  return renderToStaticMarkup(
    <UpstreamAccountUsageCard
      title="7d window"
      description="Secondary weekly limit."
      window={{
        usedPercent: 18,
        usedText: "18 requests",
        limitText: "500 requests",
        resetsAt: "2026-03-18T00:00:00.000Z",
        windowDurationMins: 10080,
      }}
      history={[]}
      historyKey="secondaryUsedPercent"
      emptyLabel={emptyLabel}
      accentClassName="text-info"
      {...overrides}
    />,
  );
}

describe("UpstreamAccountUsageCard", () => {
  it("renders weak ASCII placeholders when the window snapshot is missing", () => {
    const html = renderCard({
      window: null,
      history,
    });

    expect((html.match(/>-</g) ?? []).length).toBeGreaterThanOrEqual(5);
    expect(html).toContain("text-base-content/55");
    expect(html).not.toContain(emptyLabel);
  });

  it("keeps the empty history copy for known snapshots without historical samples", () => {
    const html = renderCard();

    expect(html).toContain("18 requests");
    expect(html).toContain("500 requests");
    expect(html).toContain(emptyLabel);
  });

  it("keeps real zero-percent snapshots out of placeholder mode", () => {
    const html = renderCard({
      window: {
        usedPercent: 0,
        usedText: "0 requests",
        limitText: "500 requests",
        resetsAt: "2026-03-18T00:00:00.000Z",
        windowDurationMins: 10080,
      },
    });

    expect(html).toContain(">0%</span>");
    expect(html).toContain("0 requests");
    expect((html.match(/>-</g) ?? []).length).toBe(0);
    expect(html).toContain(emptyLabel);
  });
});

describe("ReserveQuotaCard", () => {
  const labels = {
    available: "available",
    unavailable: "unavailable",
    unknown: "unknown",
    used: "used",
    availablePercent: "available",
    reset: "Reset",
    window: "min window",
    capturedAt: "Captured",
    unknownValue: "Unknown",
  };

  it("renders structured Reserve status without exposing an absolute balance", () => {
    const html = renderToStaticMarkup(
      <ReserveQuotaCard
        title="Reserve"
        description="Structured Reserve quota."
        snapshot={{
          availability: "available",
          primaryWindow: {
            usedPercent: 12,
            availablePercent: 88,
            usedText: "12% used",
            limitText: "Reserve window",
            resetsAt: "2026-03-11T18:00:00.000Z",
            windowDurationMins: 300,
          },
          capturedAt: "2026-03-11T12:30:00.000Z",
        }}
        labels={labels}
      />,
    );

    expect(html).toContain("available");
    expect(html).toContain("12%");
    expect(html).toContain("88%");
    expect(html).toContain("Captured");
    expect(html).not.toContain("balance");
  });

  it("uses unknown status when the Reserve snapshot is absent", () => {
    const html = renderToStaticMarkup(
      <ReserveQuotaCard
        title="Reserve"
        description="Structured Reserve quota."
        snapshot={null}
        labels={labels}
      />,
    );

    expect(html).toContain("unknown");
    expect(html).toContain("Unknown");
    expect(html).not.toContain("unavailable");
  });
});
