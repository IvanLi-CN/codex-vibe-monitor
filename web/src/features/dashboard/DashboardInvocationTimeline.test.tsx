import { describe, expect, it } from "vitest";
import type { InvocationTimelineRecord } from "../../lib/api";
import {
  assignInvocationTimelineLanes,
  getInvocationTimelineLaneCount,
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

  it("keeps unknown terminal duration from freeing the lane", () => {
    const lanes = assignInvocationTimelineLanes(
      [
        record("invoke-6", "2026-03-26T12:00:00.000Z", null),
        record("invoke-7", "2026-03-26T12:00:01.000Z", 1_000),
      ],
      "2026-03-26T12:00:05.000Z",
      Date.parse("2026-03-26T12:00:05.000Z"),
    );

    expect(lanes.map((item) => item.lane)).toEqual([0, 1]);
    expect(lanes[0].endMs).toBeGreaterThan(lanes[0].startMs);
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
