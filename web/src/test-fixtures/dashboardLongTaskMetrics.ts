export type LongTaskEntry = {
  startTime: number;
  duration: number;
};

export type DashboardPhaseMetrics = {
  longTaskCount: number;
  maxLongTaskMs: number;
  over50msCount: number;
  totalBlockingTimeMs: number;
  p95LongTaskMs: number;
};

export function summarizeLongTasks(
  entries: LongTaskEntry[],
  startTime: number,
  endTime: number,
): DashboardPhaseMetrics {
  const durations = entries
    .filter((entry) => entry.startTime < endTime && entry.startTime + entry.duration > startTime)
    .map((entry) => entry.duration)
    .sort((left, right) => left - right);
  const p95Index = Math.max(0, Math.ceil(durations.length * 0.95) - 1);

  return {
    longTaskCount: durations.length,
    maxLongTaskMs: durations.length > 0 ? Math.max(...durations) : 0,
    over50msCount: durations.filter((duration) => duration > 50).length,
    totalBlockingTimeMs: durations.reduce(
      (total, duration) => total + Math.max(0, duration - 50),
      0,
    ),
    p95LongTaskMs: durations[p95Index] ?? 0,
  };
}
