import { describe, expect, it } from "vitest";
import type { InvocationTimelineRecord, InvocationTimelineResponse } from "../../lib/api";
import {
  assignInvocationTimelineLanes,
  getInvocationTimelineLaneCount,
  resolveInvocationTimelineLayout,
  shouldFallbackForInvalidTimelineBounds,
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

describe("shouldFallbackForInvalidTimelineBounds", () => {
  it("falls back when a response exists without valid bounds", () => {
    const response = {
      rangeStart: "not-a-date",
      rangeEnd: "2026-03-26T12:00:00.000Z",
      bucketSeconds: 300,
      points: [],
    };

    expect(shouldFallbackForInvalidTimelineBounds(response, null, null)).toBe(true);
    expect(shouldFallbackForInvalidTimelineBounds(response, { startMs: 1, endMs: 2 }, null)).toBe(
      false,
    );
    expect(
      shouldFallbackForInvalidTimelineBounds(response, null, {} as InvocationTimelineResponse),
    ).toBe(false);
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

  it("adapts lane height while respecting the 8px and 16px bounds", () => {
    expect(resolveInvocationTimelineLayout(20, false).laneHeight).toBe(13);
    expect(resolveInvocationTimelineLayout(100, true).laneHeight).toBe(8);
    expect(resolveInvocationTimelineLayout(2, true).laneHeight).toBe(16);
    expect(resolveInvocationTimelineLayout(2, true).chartHeightPx).toBe(336);
  });

  it("keeps a 190-lane workload inside the fixed chart viewport", () => {
    const layout = resolveInvocationTimelineLayout(190, false);

    expect(layout.visibleLaneCount).toBe(190);
    expect(layout.chartHeightPx).toBe(320);
    expect(layout.laneAreaHeightPx).toBe(292);
    expect(layout.laneHeight).toBe(8);
    expect(layout.laneStep).toBe(9);
    expect(layout.lanePlotHeight).toBe(1_733);
    expect(layout.lanePlotHeight).toBeGreaterThan(layout.laneAreaHeightPx);
  });
});
