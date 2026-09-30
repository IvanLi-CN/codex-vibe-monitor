import { describe, expect, it } from "vitest";
import type { InvocationTimelineRecord, InvocationTimelineResponse } from "../../lib/api";
import {
  assignInvocationTimelineLanes,
  getInvocationTimelineLaneCount,
  hasInvocationTimelineRefreshError,
  resolveInvocationTimelineLayout,
  resolveInvocationTimelineScrollTop,
  resolveInvocationTimelineTooltipPosition,
  resolveVisibleInvocationLaneRange,
  shouldAdvanceInvocationTimelineBars,
  shouldShowTimelineUnavailable,
} from "./DashboardInvocationTimeline";

function record(
  invokeId: string,
  occurredAt: string,
  tTotalMs: number | null,
  isInFlight = false,
): InvocationTimelineRecord {
  return {
    id: Number(invokeId.replace(/\D/g, "")) || 1,
    invokeId,
    occurredAt,
    endAt: tTotalMs == null ? null : new Date(Date.parse(occurredAt) + tTotalMs).toISOString(),
    isInFlight,
    tTotalMs,
  };
}

describe("assignInvocationTimelineLanes", () => {
  it("uses the lowest idle lane and keeps zero duration visible", () => {
    const lanes = assignInvocationTimelineLanes(
      [
        record("invoke-1", "2026-03-26T12:00:00.000Z", 1_000),
        record("invoke-2", "2026-03-26T12:00:00.500Z", 1_000),
        record("invoke-3", "2026-03-26T12:00:01.000Z", 0),
      ],
      "2026-03-26T12:00:02.000Z",
      Date.parse("2026-03-26T12:00:02.000Z"),
    );

    expect(lanes.map((item) => item.lane)).toEqual([0, 1, 0]);
    expect(lanes[2].endMs).toBeGreaterThan(lanes[2].startMs);
  });

  it("assigns dense concurrent calls without losing the lowest available lane", () => {
    const startMs = Date.parse("2026-03-26T12:00:00.000Z");
    const records = Array.from({ length: 550 }, (_, index) =>
      record(`invoke-${index}`, new Date(startMs + index).toISOString(), 120_000),
    );
    records.push(record("invoke-after-spike", new Date(startMs + 121_000).toISOString(), 1_000));

    const lanes = assignInvocationTimelineLanes(
      records,
      new Date(startMs + 122_000).toISOString(),
      startMs + 122_000,
    );

    expect(getInvocationTimelineLaneCount(lanes)).toBe(550);
    expect(lanes.at(-1)?.lane).toBe(0);
  });

  it("extends an in-flight bar to the current clock", () => {
    const lanes = assignInvocationTimelineLanes(
      [record("invoke-4", "2026-03-26T12:00:00.000Z", null, true)],
      "2026-03-26T12:00:01.000Z",
      Date.parse("2026-03-26T12:00:05.000Z"),
    );

    expect(lanes[0].endMs).toBe(Date.parse("2026-03-26T12:00:05.000Z"));
  });

  it("freezes live extension while disconnected", () => {
    const lanes = assignInvocationTimelineLanes(
      [record("invoke-5", "2026-03-26T12:00:00.000Z", null, true)],
      "2026-03-26T12:00:01.000Z",
      Date.parse("2026-03-26T12:00:05.000Z"),
      false,
    );

    expect(lanes[0].endMs).toBe(Date.parse("2026-03-26T12:00:01.000Z"));
  });

  it("freezes an opaque snapshot at its successful render time", () => {
    const snapshotAtMs = Date.parse("2026-03-26T12:00:01.000Z");
    const lanes = assignInvocationTimelineLanes(
      [record("invoke-opaque", "2026-03-26T12:00:00.000Z", null, true)],
      "opaque-snapshot-token",
      Date.parse("2026-03-26T12:00:05.000Z"),
      false,
      snapshotAtMs,
    );

    expect(lanes[0].endMs).toBe(snapshotAtMs);
  });

  it("shows unknown terminal duration without treating it as still running", () => {
    const unknown = {
      ...record("invoke-6", "2026-03-26T12:00:00.000Z", null),
      endAt: "2026-03-26T12:00:04.000Z",
    };
    const lanes = assignInvocationTimelineLanes(
      [unknown, record("invoke-7", "2026-03-26T12:00:01.000Z", 1_000)],
      "2026-03-26T12:00:05.000Z",
      Date.parse("2026-03-26T12:00:05.000Z"),
    );

    expect(lanes.map((item) => item.lane)).toEqual([0, 0]);
    expect(lanes[0].endMs).toBeGreaterThan(lanes[0].startMs);
  });

  it("uses valid total duration when the terminal timestamp is absent", () => {
    const item = {
      ...record("invoke-8", "2026-03-26T12:00:00.000Z", 90_000),
      endAt: null,
    };

    const lanes = assignInvocationTimelineLanes(
      [item],
      "2026-03-26T12:05:00.000Z",
      Date.parse("2026-03-26T12:05:00.000Z"),
    );

    expect(lanes[0].endMs).toBe(Date.parse("2026-03-26T12:01:30.000Z"));
  });

  it("keeps the full height after an early concurrency spike", () => {
    const lanes = assignInvocationTimelineLanes(
      [
        record("invoke-a", "2026-03-26T12:00:00.000Z", 3_600_000),
        record("invoke-b", "2026-03-26T12:00:00.100Z", 3_600_000),
        record("invoke-c", "2026-03-26T12:00:00.200Z", 3_600_000),
        record("invoke-d", "2026-03-26T13:00:00.000Z", 1_000),
      ],
      "2026-03-26T13:00:01.000Z",
    );

    expect(lanes.map((item) => item.lane)).toEqual([0, 1, 2, 0]);
    expect(getInvocationTimelineLaneCount(lanes)).toBe(3);
  });
});

describe("shouldShowTimelineUnavailable", () => {
  it("keeps the new timeline surface unavailable when bounds are invalid", () => {
    const response = {
      rangeStart: "not-a-date",
      rangeEnd: "2026-03-26T12:00:00.000Z",
      bucketSeconds: 300,
      points: [],
    };

    expect(shouldShowTimelineUnavailable(response, null, null)).toBe(true);
    expect(shouldShowTimelineUnavailable(response, { startMs: 1, endMs: 2 }, null)).toBe(false);
    expect(shouldShowTimelineUnavailable(response, null, {} as InvocationTimelineResponse)).toBe(
      false,
    );
  });
});

describe("shouldAdvanceInvocationTimelineBars", () => {
  it("advances only connected live timelines without mock overrides", () => {
    expect(shouldAdvanceInvocationTimelineBars(false, true, false)).toBe(true);
    expect(shouldAdvanceInvocationTimelineBars(true, true, false)).toBe(false);
    expect(shouldAdvanceInvocationTimelineBars(false, false, false)).toBe(false);
    expect(shouldAdvanceInvocationTimelineBars(false, true, true)).toBe(false);
    expect(shouldAdvanceInvocationTimelineBars(false, true, false, true)).toBe(false);
  });
});

describe("hasInvocationTimelineRefreshError", () => {
  it("surfaces a closed-day refresh failure while retaining the last timeline snapshot", () => {
    expect(hasInvocationTimelineRefreshError("yesterday refresh failed", null, false)).toBe(true);
    expect(hasInvocationTimelineRefreshError(null, null, false)).toBe(false);
    expect(hasInvocationTimelineRefreshError("fixture error", null, true)).toBe(false);
  });
});

describe("resolveInvocationTimelineLayout", () => {
  it("keeps four visual lanes and the original chart height for sparse data", () => {
    const layout = resolveInvocationTimelineLayout(1, false);

    expect(layout.visibleLaneCount).toBe(4);
    expect(layout.chartHeightPx).toBe(320);
    expect(layout.laneHeight).toBe(16);
    expect(layout.laneStep - layout.laneHeight).toBe(1);
    expect(layout.lanePlotHeight).toBe(292);
  });

  it("adapts lane height within the readable range and keeps one-pixel gaps", () => {
    expect(resolveInvocationTimelineLayout(20, false).laneHeight).toBe(13);
    expect(resolveInvocationTimelineLayout(20, false).laneGap).toBe(1);
    expect(resolveInvocationTimelineLayout(100, true).laneHeight).toBe(8);
    expect(resolveInvocationTimelineLayout(100, true).laneGap).toBe(1);
    expect(resolveInvocationTimelineLayout(300, false).laneHeight).toBe(8);
    expect(resolveInvocationTimelineLayout(300, false).laneGap).toBe(1);
    expect(resolveInvocationTimelineLayout(2, true).laneHeight).toBe(16);
    expect(resolveInvocationTimelineLayout(2, true).chartHeightPx).toBe(336);
  });

  it("keeps 190 lanes readable and scrolls inside the fixed chart viewport", () => {
    const layout = resolveInvocationTimelineLayout(190, false);

    expect(layout.visibleLaneCount).toBe(190);
    expect(layout.chartHeightPx).toBe(320);
    expect(layout.laneAreaHeightPx).toBe(292);
    expect(layout.laneHeight).toBe(8);
    expect(layout.laneGap).toBe(1);
    expect(layout.lanePlotHeight).toBeGreaterThan(layout.laneAreaHeightPx);
    expect(layout.laneContentHeight).toBe(190 * 8 + 189);
  });

  it("preserves the minimum lane height and gap while dense rows overflow", () => {
    const layout = resolveInvocationTimelineLayout(360, false);

    expect(layout.chartHeightPx).toBe(320);
    expect(layout.laneHeight).toBe(8);
    expect(layout.laneGap).toBe(1);
    expect(layout.lanePlotHeight).toBeGreaterThan(layout.laneAreaHeightPx);
  });
});

describe("resolveInvocationTimelineTooltipPosition", () => {
  it("places the tooltip beside a central pointer", () => {
    expect(
      resolveInvocationTimelineTooltipPosition(
        { x: 100, y: 80 },
        { width: 400, height: 280 },
        { width: 150, height: 72 },
      ),
    ).toEqual({ x: 112, y: 92 });
  });

  it("flips and clamps the tooltip inside the plot at its edges", () => {
    expect(
      resolveInvocationTimelineTooltipPosition(
        { x: 390, y: 275 },
        { width: 400, height: 280 },
        { width: 150, height: 72 },
      ),
    ).toEqual({ x: 228, y: 191 });
    expect(
      resolveInvocationTimelineTooltipPosition(
        { x: 0, y: 0 },
        { width: 400, height: 280 },
        { width: 150, height: 72 },
      ),
    ).toEqual({ x: 12, y: 12 });
  });
});

describe("resolveInvocationTimelineScrollTop", () => {
  it("starts dense views at the bottom and preserves manual position on refresh", () => {
    expect(resolveInvocationTimelineScrollTop(false, 0, 120)).toBe(120);
    expect(resolveInvocationTimelineScrollTop(false, 48, 120)).toBe(120);
    expect(resolveInvocationTimelineScrollTop(true, 48, 120)).toBe(48);
    expect(resolveInvocationTimelineScrollTop(true, 180, 120)).toBe(120);
  });
});

describe("resolveVisibleInvocationLaneRange", () => {
  it("renders only the viewport and overscan while preserving both scroll boundaries", () => {
    const layout = resolveInvocationTimelineLayout(550, false);
    const bottom = resolveVisibleInvocationLaneRange(
      550,
      layout.laneStep,
      layout.lanePlotHeight,
      layout.laneAreaHeightPx,
      layout.lanePlotHeight - layout.laneAreaHeightPx,
    );
    const top = resolveVisibleInvocationLaneRange(
      550,
      layout.laneStep,
      layout.lanePlotHeight,
      layout.laneAreaHeightPx,
      0,
    );

    const visibleRows = Math.ceil(layout.laneAreaHeightPx / layout.laneStep);
    expect(bottom.firstLane).toBe(0);
    expect(bottom.lastLane).toBeGreaterThan(visibleRows - 1);
    expect(bottom.lastLane).toBeLessThan(visibleRows + 2);
    expect(top.firstLane).toBeGreaterThan(250);
    expect(top.lastLane).toBe(549);
    expect(top.lastLane - top.firstLane + 1).toBeLessThan(visibleRows + 4);
  });
});
