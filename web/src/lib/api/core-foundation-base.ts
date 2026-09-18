import { getBrowserTimeZone } from "../timeZone";
import { normalizeBlockedBindingDiagnostic } from "./core-foundation-normalizers-base";
import type { BlockedBindingDiagnostic } from "./core-foundation-record-types";
import type { ForwardProxyValidationKind } from "./core-foundation-settings-types";

const rawBase =
  import.meta.env.VITE_APP_RUNTIME === "demo"
    ? import.meta.env.BASE_URL
    : (import.meta.env.VITE_API_BASE_URL ?? "");
const API_BASE = rawBase.endsWith("/") ? rawBase.slice(0, -1) : rawBase;
const FORWARD_PROXY_VALIDATION_TIMEOUT_MS = 5_000;
const FORWARD_PROXY_SUBSCRIPTION_VALIDATION_TIMEOUT_MS = 60_000;
const FORWARD_PROXY_HISTORY_DAY_MS = 86_400_000;
export const DEFAULT_POOL_ROUTING_MAINTENANCE_SETTINGS = {
  primarySyncIntervalSecs: 300,
  secondarySyncIntervalSecs: 1_800,
  priorityAvailableAccountCap: 100,
} as const;

export const DEFAULT_PROMPT_CACHE_CONVERSATION_LIMIT = 50;
export const DEFAULT_STICKY_KEY_CONVERSATION_LIMIT = 50;

type ZonedDateParts = {
  year: number;
  month: number;
  day: number;
  weekday: number;
};

export const withBase = (path: string) => `${API_BASE}${path}`;

export class ApiRequestError extends Error {
  readonly status: number;
  readonly code?: string;
  readonly blockedBinding?: BlockedBindingDiagnostic | null;

  constructor(
    status: number,
    message: string,
    options?: {
      code?: string;
      blockedBinding?: BlockedBindingDiagnostic | null;
    },
  ) {
    super(message);
    this.name = "ApiRequestError";
    this.status = status;
    this.code = options?.code;
    this.blockedBinding = options?.blockedBinding ?? null;
    Object.setPrototypeOf(this, new.target.prototype);
  }
}

export function buildRequestError(response: Response, rawText: string): ApiRequestError {
  const compactText = rawText.replace(/\s+/g, " ").trim();
  const detail = (compactText || response.statusText || "").slice(0, 220);
  let parsedCode: string | undefined;
  let parsedBlockedBinding: BlockedBindingDiagnostic | null = null;
  try {
    const payload = JSON.parse(rawText) as Record<string, unknown>;
    parsedCode =
      typeof payload.code === "string" && payload.code.trim() ? payload.code.trim() : undefined;
    parsedBlockedBinding = normalizeBlockedBindingDiagnostic(payload.blockedBinding);
  } catch {
    parsedCode = undefined;
    parsedBlockedBinding = null;
  }
  return new ApiRequestError(
    response.status,
    detail ? `Request failed: ${response.status} ${detail}` : `Request failed: ${response.status}`,
    {
      code: parsedCode,
      blockedBinding: parsedBlockedBinding,
    },
  );
}

export async function fetchJson<T>(path: string, init?: RequestInit): Promise<T> {
  const { data } = await fetchJsonResponse<T>(path, init);
  return data;
}

async function fetchJsonResponse<T>(
  path: string,
  init?: RequestInit,
): Promise<{ data: T; response: Response }> {
  const response = await fetch(withBase(path), {
    headers: {
      "Content-Type": "application/json",
    },
    ...init,
  });

  if (!response.ok) {
    const rawText = await response.text();
    throw buildRequestError(response, rawText);
  }

  if (response.status === 204) {
    return { data: undefined as T, response };
  }

  const rawText = await response.text();
  if (!rawText.trim()) {
    return { data: undefined as T, response };
  }

  return {
    data: JSON.parse(rawText) as T,
    response,
  };
}

export async function ensureJsonRequestOk(response: Response): Promise<void> {
  if (response.ok) {
    return;
  }

  const rawText = await response.text();
  throw buildRequestError(response, rawText);
}

export function parseForwardProxyHistoryRangeSeconds(range: string): number | null {
  if (range.endsWith("mo")) {
    const value = Number(range.slice(0, -2));
    return Number.isFinite(value) ? value * 30 * 86_400 : null;
  }
  const unit = range.slice(-1);
  const value = Number(range.slice(0, -1));
  if (!Number.isFinite(value)) return null;
  switch (unit) {
    case "d":
      return value * 86_400;
    case "h":
      return value * 3_600;
    case "m":
      return value * 60;
    default:
      return null;
  }
}

export function getForwardProxyHistoryOffsetMinutes(date: Date, timeZone: string): number | null {
  const timeZoneName = new Intl.DateTimeFormat("en-US", {
    timeZone,
    timeZoneName: "shortOffset",
    hour: "2-digit",
  })
    .formatToParts(date)
    .find((part) => part.type === "timeZoneName")?.value;
  const normalized = (timeZoneName ?? "").replace(/^UTC/, "GMT");
  if (!normalized || normalized === "GMT") {
    return 0;
  }
  const match = normalized.match(/^GMT([+-])(\d{1,2})(?::(\d{2}))?$/i);
  if (!match) {
    return null;
  }
  const sign = match[1] === "-" ? -1 : 1;
  const hours = Number(match[2] ?? "0");
  const minutes = Number(match[3] ?? "0");
  return sign * (hours * 60 + minutes);
}

export function getForwardProxyHistoryDateParts(
  date: Date,
  timeZone: string,
): ZonedDateParts | null {
  const parts = new Intl.DateTimeFormat("en-US", {
    timeZone,
    weekday: "short",
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
  }).formatToParts(date);
  const partMap = Object.fromEntries(
    parts
      .filter((part) => ["weekday", "year", "month", "day"].includes(part.type))
      .map((part) => [part.type, part.value]),
  );
  const weekdayMap: Record<string, number> = {
    Mon: 0,
    Tue: 1,
    Wed: 2,
    Thu: 3,
    Fri: 4,
    Sat: 5,
    Sun: 6,
  };
  const year = Number(partMap.year);
  const month = Number(partMap.month);
  const day = Number(partMap.day);
  const weekday = weekdayMap[partMap.weekday ?? ""];
  if (
    !Number.isFinite(year) ||
    !Number.isFinite(month) ||
    !Number.isFinite(day) ||
    weekday === undefined
  ) {
    return null;
  }
  return { year, month, day, weekday };
}

export function addUtcDays(parts: ZonedDateParts, days: number): ZonedDateParts {
  const shifted = new Date(Date.UTC(parts.year, parts.month - 1, parts.day + days));
  return {
    year: shifted.getUTCFullYear(),
    month: shifted.getUTCMonth() + 1,
    day: shifted.getUTCDate(),
    weekday: shifted.getUTCDay(),
  };
}

export function forwardProxyHistoryLocalMidnightUtcMillis(
  timeZone: string,
  parts: Pick<ZonedDateParts, "year" | "month" | "day">,
): number {
  const localMidnightUtc = Date.UTC(parts.year, parts.month - 1, parts.day, 0, 0, 0);
  let candidate = localMidnightUtc;
  for (let index = 0; index < 4; index += 1) {
    const offsetMinutes = getForwardProxyHistoryOffsetMinutes(new Date(candidate), timeZone);
    if (offsetMinutes === null) {
      break;
    }
    const adjusted = localMidnightUtc - offsetMinutes * 60_000;
    if (adjusted === candidate) {
      return candidate;
    }
    candidate = adjusted;
  }
  return candidate;
}

export function resolveForwardProxyHistoryRangeMillis(
  range: string,
  timeZone: string,
  now: Date,
): { startMs: number; endMs: number } | null {
  const localNow = getForwardProxyHistoryDateParts(now, timeZone);
  if (!localNow) {
    return null;
  }

  if (range === "today") {
    return {
      startMs: forwardProxyHistoryLocalMidnightUtcMillis(timeZone, localNow),
      endMs: now.getTime(),
    };
  }
  if (range === "thisWeek") {
    const weekStart = addUtcDays(localNow, -localNow.weekday);
    return {
      startMs: forwardProxyHistoryLocalMidnightUtcMillis(timeZone, weekStart),
      endMs: now.getTime(),
    };
  }
  if (range === "thisMonth") {
    return {
      startMs: forwardProxyHistoryLocalMidnightUtcMillis(timeZone, {
        year: localNow.year,
        month: localNow.month,
        day: 1,
      }),
      endMs: now.getTime(),
    };
  }

  const durationSeconds = parseForwardProxyHistoryRangeSeconds(range);
  if (durationSeconds === null) {
    return null;
  }
  return {
    startMs: now.getTime() - durationSeconds * 1_000,
    endMs: now.getTime(),
  };
}

export function resolveForwardProxyHistoryTimeZone(range: string, timeZone?: string): string {
  const candidate = timeZone ?? getBrowserTimeZone();
  try {
    const rangeWindow = resolveForwardProxyHistoryRangeMillis(range, candidate, new Date());
    if (!rangeWindow) {
      return candidate;
    }
    for (
      let currentMs = rangeWindow.startMs;
      currentMs < rangeWindow.endMs;
      currentMs += FORWARD_PROXY_HISTORY_DAY_MS
    ) {
      const offsetMinutes = getForwardProxyHistoryOffsetMinutes(new Date(currentMs), candidate);
      if (offsetMinutes !== null && offsetMinutes % 60 !== 0) {
        throw new Error(
          `unsupported timeZone for forward proxy hourly timeseries: ${candidate}; hourly buckets require whole-hour UTC offsets`,
        );
      }
    }
    const lastSampleMs = Math.max(rangeWindow.startMs, rangeWindow.endMs - 1);
    const lastOffsetMinutes = getForwardProxyHistoryOffsetMinutes(
      new Date(lastSampleMs),
      candidate,
    );
    if (lastOffsetMinutes !== null && lastOffsetMinutes % 60 !== 0) {
      throw new Error(
        `unsupported timeZone for forward proxy hourly timeseries: ${candidate}; hourly buckets require whole-hour UTC offsets`,
      );
    }
    return candidate;
  } catch (error) {
    if (
      error instanceof Error &&
      error.message.startsWith("unsupported timeZone for forward proxy hourly timeseries:")
    ) {
      throw error;
    }
    return candidate;
  }
}

export function forwardProxyValidationTimeoutMs(kind: ForwardProxyValidationKind): number {
  return kind === "subscriptionUrl"
    ? FORWARD_PROXY_SUBSCRIPTION_VALIDATION_TIMEOUT_MS
    : FORWARD_PROXY_VALIDATION_TIMEOUT_MS;
}
