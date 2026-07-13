import { describe, expect, it } from "vitest";
import { buildDashboardTodayRateSnapshot } from "./dashboardTodayRateSnapshot";

function minutePoint(offsetMinutes: number, totalTokens: number, totalCost: number) {
  const bucketStart = new Date(2026, 3, 10, 0, offsetMinutes, 0, 0);
  const bucketEnd = new Date(bucketStart.getTime() + 60_000);
  return {
    bucketStart: formatLocal(bucketStart),
    bucketEnd: formatLocal(bucketEnd),
    totalCount: 1,
    successCount: 1,
    failureCount: 0,
    totalTokens,
    totalCost,
  };
}

describe("buildDashboardTodayRateSnapshot", () => {
  it("uses the active tail inside the latest five-minute window as the default rate source", () => {
    const snapshot = buildDashboardTodayRateSnapshot(
      {
        rangeStart: "2026-04-10 00:00:00",
        rangeEnd: "2026-04-10 00:06:30",
        bucketSeconds: 60,
        points: [
          minutePoint(1, 600, 0.06),
          minutePoint(2, 800, 0.08),
          minutePoint(3, 1000, 0.1),
          minutePoint(4, 1200, 0.12),
          minutePoint(5, 1400, 0.14),
          minutePoint(6, 5000, 0.5),
        ],
      },
      { now: new Date(2026, 3, 10, 0, 6, 30, 0) },
    );

    expect(snapshot?.tokensPerMinute).toBe(2000);
    expect(snapshot?.spendRate).toBeCloseTo(0.2, 6);
    expect(snapshot?.windowMinutes).toBe(5);
    expect(snapshot?.available).toBe(true);
  });

  it("does not let leading zero minutes dilute the active tail average", () => {
    const snapshot = buildDashboardTodayRateSnapshot(
      {
        rangeStart: "2026-04-10 00:00:00",
        rangeEnd: "2026-04-10 00:05:30",
        bucketSeconds: 60,
        points: [
          minutePoint(1, 0, 0),
          minutePoint(2, 0, 0),
          minutePoint(3, 0, 0),
          minutePoint(4, 900, 0.09),
          minutePoint(5, 600, 0.06),
        ],
      },
      { now: new Date(2026, 3, 10, 0, 5, 30, 0), targetWindowMinutes: 5 },
    );

    expect(snapshot?.tokensPerMinute).toBe(1000);
    expect(snapshot?.spendRate).toBeCloseTo(0.1, 6);
    expect(snapshot?.windowMinutes).toBe(1.5);
  });

  it("uses separate active tails when tokens and cost start in different buckets", () => {
    const snapshot = buildDashboardTodayRateSnapshot(
      {
        rangeStart: "2026-04-10 00:00:00",
        rangeEnd: "2026-04-10 00:05:00",
        bucketSeconds: 60,
        points: [
          minutePoint(1, 600, 0),
          minutePoint(2, 0, 0),
          minutePoint(3, 0, 0.12),
          minutePoint(4, 0, 0.08),
        ],
      },
      { now: new Date(2026, 3, 10, 0, 5, 0, 0) },
    );

    expect(snapshot?.tokensPerMinute).toBe(150);
    expect(snapshot?.spendRate).toBeCloseTo(0.1, 6);
    expect(snapshot?.windowMinutes).toBe(4);
  });

  it("counts the active bucket that overlaps the rolling window start", () => {
    const snapshot = buildDashboardTodayRateSnapshot(
      {
        rangeStart: "2026-04-10 00:00:00",
        rangeEnd: "2026-04-10 00:05:30",
        bucketSeconds: 60,
        points: [minutePoint(0, 500, 0.05), minutePoint(1, 500, 0.05)],
      },
      { now: new Date(2026, 3, 10, 0, 5, 30, 0), targetWindowMinutes: 5 },
    );

    expect(snapshot?.tokensPerMinute).toBe(200);
    expect(snapshot?.spendRate).toBeCloseTo(0.02, 6);
    expect(snapshot?.windowMinutes).toBe(5);
  });

  it("includes the current partial minute and divides by the actual active elapsed minutes", () => {
    const snapshot = buildDashboardTodayRateSnapshot(
      {
        rangeStart: "2026-04-10 00:00:00",
        rangeEnd: "2026-04-10 00:03:10",
        bucketSeconds: 60,
        points: [minutePoint(0, 600, 0.06), minutePoint(1, 900, 0.09), minutePoint(2, 1500, 0.15)],
      },
      { now: new Date(2026, 3, 10, 0, 3, 10, 0) },
    );

    expect(snapshot?.tokensPerMinute).toBeCloseTo(3000 / (19 / 6), 6);
    expect(snapshot?.spendRate).toBeCloseTo(0.3 / (19 / 6), 6);
    expect(snapshot?.windowMinutes).toBeCloseTo(19 / 6, 6);
  });

  it("keeps post-activity quiet time in the denominator to avoid spikes", () => {
    const snapshot = buildDashboardTodayRateSnapshot(
      {
        rangeStart: "2026-04-10 00:00:00",
        rangeEnd: "2026-04-10 00:05:40",
        bucketSeconds: 60,
        points: [
          minutePoint(1, 1200, 0.12),
          minutePoint(2, 0, 0),
          minutePoint(3, 0, 0),
          minutePoint(4, 0, 0),
          minutePoint(5, 0, 0),
        ],
      },
      { now: new Date(2026, 3, 10, 0, 5, 40, 0) },
    );

    expect(snapshot?.tokensPerMinute).toBeCloseTo(1200 / (14 / 3), 6);
    expect(snapshot?.spendRate).toBeCloseTo(0.12 / (14 / 3), 6);
    expect(snapshot?.windowMinutes).toBeCloseTo(14 / 3, 6);
  });

  it("uses current same-day time so active-tail rates decay during quiet periods", () => {
    const snapshot = buildDashboardTodayRateSnapshot(
      {
        rangeStart: "2026-04-10 00:00:00",
        rangeEnd: "2026-04-10 00:02:00",
        bucketSeconds: 60,
        points: [minutePoint(1, 1200, 0.12)],
      },
      { now: new Date(2026, 3, 10, 0, 5, 0, 0) },
    );

    expect(snapshot?.tokensPerMinute).toBe(300);
    expect(snapshot?.spendRate).toBeCloseTo(0.03, 6);
    expect(snapshot?.windowMinutes).toBe(4);
  });

  it("keeps fixed fixture days anchored to their response end instead of wall-clock now", () => {
    const snapshot = buildDashboardTodayRateSnapshot(
      {
        rangeStart: "2026-04-10 00:00:00",
        rangeEnd: "2026-04-10 00:02:00",
        bucketSeconds: 60,
        points: [minutePoint(1, 1200, 0.12)],
      },
      { now: new Date(2026, 3, 11, 0, 5, 0, 0) },
    );

    expect(snapshot?.tokensPerMinute).toBe(1200);
    expect(snapshot?.spendRate).toBeCloseTo(0.12, 6);
    expect(snapshot?.windowMinutes).toBe(1);
  });

  it("returns zero values when there are no completed minutes yet", () => {
    const snapshot = buildDashboardTodayRateSnapshot(
      {
        rangeStart: "2026-04-10 00:00:00",
        rangeEnd: "2026-04-10 00:00:20",
        bucketSeconds: 60,
        points: [],
      },
      { now: new Date(2026, 3, 10, 0, 0, 20, 0) },
    );

    expect(snapshot?.tokensPerMinute).toBe(0);
    expect(snapshot?.spendRate).toBe(0);
    expect(snapshot?.windowMinutes).toBeCloseTo(1 / 3, 6);
    expect(snapshot?.available).toBe(true);
  });

  it("keeps yesterday closed-day rates on the previous natural day even when rangeEnd is rounded into the next minute", () => {
    const snapshot = buildDashboardTodayRateSnapshot(
      {
        rangeStart: "2026-04-09 00:00:00",
        rangeEnd: "2026-04-10 00:01:00",
        bucketSeconds: 60,
        points: [
          {
            bucketStart: "2026-04-09 23:55:00",
            bucketEnd: "2026-04-09 23:56:00",
            totalCount: 1,
            successCount: 1,
            failureCount: 0,
            totalTokens: 500,
            totalCost: 0.05,
          },
          {
            bucketStart: "2026-04-09 23:56:00",
            bucketEnd: "2026-04-09 23:57:00",
            totalCount: 1,
            successCount: 1,
            failureCount: 0,
            totalTokens: 600,
            totalCost: 0.06,
          },
          {
            bucketStart: "2026-04-09 23:57:00",
            bucketEnd: "2026-04-09 23:58:00",
            totalCount: 1,
            successCount: 1,
            failureCount: 0,
            totalTokens: 700,
            totalCost: 0.07,
          },
          {
            bucketStart: "2026-04-09 23:58:00",
            bucketEnd: "2026-04-09 23:59:00",
            totalCount: 1,
            successCount: 1,
            failureCount: 0,
            totalTokens: 800,
            totalCost: 0.08,
          },
          {
            bucketStart: "2026-04-09 23:59:00",
            bucketEnd: "2026-04-10 00:00:00",
            totalCount: 1,
            successCount: 1,
            failureCount: 0,
            totalTokens: 900,
            totalCost: 0.09,
          },
        ],
      },
      {
        now: new Date(2026, 3, 10, 12, 0, 0, 0),
        closedNaturalDay: true,
      },
    );

    expect(snapshot?.tokensPerMinute).toBe(700);
    expect(snapshot?.spendRate).toBeCloseTo(0.07, 6);
    expect(snapshot?.windowMinutes).toBe(5);
    expect(snapshot?.available).toBe(true);
  });
});

function formatLocal(date: Date) {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  const hours = String(date.getHours()).padStart(2, "0");
  const minutes = String(date.getMinutes()).padStart(2, "0");
  const seconds = String(date.getSeconds()).padStart(2, "0");
  return `${year}-${month}-${day} ${hours}:${minutes}:${seconds}`;
}
