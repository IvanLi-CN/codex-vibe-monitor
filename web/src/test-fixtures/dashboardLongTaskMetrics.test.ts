import { describe, expect, it } from "vitest";
import { summarizeLongTasks } from "./dashboardLongTaskMetrics";

describe("summarizeLongTasks", () => {
  it("counts tasks that overlap either measurement boundary", () => {
    expect(
      summarizeLongTasks(
        [
          { startTime: 80, duration: 70 },
          { startTime: 150, duration: 75 },
          { startTime: 225, duration: 60 },
        ],
        100,
        200,
      ),
    ).toEqual({
      longTaskCount: 2,
      maxLongTaskMs: 75,
      over50msCount: 2,
      totalBlockingTimeMs: 45,
      p95LongTaskMs: 75,
    });
  });

  it("excludes tasks that only touch the measurement boundaries", () => {
    expect(
      summarizeLongTasks(
        [
          { startTime: 50, duration: 50 },
          { startTime: 200, duration: 5 },
        ],
        100,
        200,
      ),
    ).toEqual({
      longTaskCount: 0,
      maxLongTaskMs: 0,
      over50msCount: 0,
      totalBlockingTimeMs: 0,
      p95LongTaskMs: 0,
    });
  });

  it("returns zero metrics for empty input", () => {
    expect(summarizeLongTasks([], 100, 200)).toEqual({
      longTaskCount: 0,
      maxLongTaskMs: 0,
      over50msCount: 0,
      totalBlockingTimeMs: 0,
      p95LongTaskMs: 0,
    });
  });

  it("sorts durations before calculating p95", () => {
    const entries = Array.from({ length: 100 }, (_, index) => ({
      startTime: 0,
      duration: 100 - index,
    }));

    expect(summarizeLongTasks(entries, 0, 101)).toMatchObject({
      longTaskCount: 100,
      maxLongTaskMs: 100,
      p95LongTaskMs: 95,
    });
  });
});
