import { getBrowserTimeZone } from "../../lib/timeZone";

export function formatManagedTaskTimestamp(value?: string | null): string {
  if (!value) return "未知";
  const timestamp = Date.parse(value);
  if (Number.isNaN(timestamp)) return "未知";
  try {
    return new Intl.DateTimeFormat("zh-CN", {
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
      hour12: false,
      timeZone: getBrowserTimeZone(),
      timeZoneName: "short",
    }).format(new Date(timestamp));
  } catch {
    return "未知";
  }
}

export function formatManagedTaskDuration(value?: number | null): string {
  if (value == null || !Number.isFinite(value) || value < 0) return "未知";
  if (value < 1_000) return `${Math.round(value)} 毫秒`;
  return `${(value / 1000).toFixed(1)} s`;
}
