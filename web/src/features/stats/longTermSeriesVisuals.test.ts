import { describe, expect, it } from "vitest";
import type { LongTermSeries } from "../../lib/api";
import { resolveLongTermSeriesVisuals } from "./longTermSeriesVisuals";

const series = (
  seriesKey: string,
  displayName: string,
  reasoningEffort?: string,
): LongTermSeries => ({ seriesKey, displayName, reasoningEffort, points: [] });

describe("resolveLongTermSeriesVisuals", () => {
  it("keeps model-family colors stable while encoding reasoning effort separately", () => {
    const items = [
      series("sol-high", "gpt-5.6-sol", "high"),
      series("sol-medium", "gpt-5.6-sol", "medium"),
      series("sol-low", "gpt-5.6-sol", "low"),
      series("terra-high", "gpt-5.6-terra", "high"),
    ];
    const visuals = resolveLongTermSeriesVisuals(items, "model", "light");

    expect(visuals.get("sol-high")).toMatchObject({ label: "gpt-5.6-sol · high" });
    expect(visuals.get("sol-medium")?.label).toBe("gpt-5.6-sol · medium");
    expect(visuals.get("sol-high")?.color).toBe(visuals.get("sol-medium")?.color);
    expect(visuals.get("sol-high")?.strokeDasharray).not.toBe(
      visuals.get("sol-medium")?.strokeDasharray,
    );
    expect(visuals.get("sol-medium")?.fillOpacity).not.toBe(visuals.get("sol-low")?.fillOpacity);
    expect(visuals.get("sol-high")?.color).not.toBe(visuals.get("terra-high")?.color);
  });

  it("assigns eight distinct model families unique colors independent of incoming order", () => {
    const items = Array.from({ length: 8 }, (_, index) =>
      series(`model-${index}`, `model-${String(index + 1).padStart(2, "0")}`, "high"),
    );
    const visuals = resolveLongTermSeriesVisuals(items, "model", "dark");
    const reordered = resolveLongTermSeriesVisuals([...items].reverse(), "model", "dark");

    expect(new Set([...visuals.values()].map((visual) => visual.color))).toHaveLength(8);
    for (const item of items) {
      expect(reordered.get(item.seriesKey)?.color).toBe(visuals.get(item.seriesKey)?.color);
    }
  });

  it("uses independent account identities and complete account labels", () => {
    const visuals = resolveLongTermSeriesVisuals(
      [
        series("account:primary", "Primary account"),
        series("account:research", "Research account"),
      ],
      "upstream",
      "light",
    );

    expect(visuals.get("account:primary")?.label).toBe("Primary account");
    expect(visuals.get("account:primary")?.color).not.toBe(visuals.get("account:research")?.color);
  });
});
