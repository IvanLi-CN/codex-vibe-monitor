import {
  ApiRequestError,
  type DashboardActivityResponse,
  type DashboardNetworkTimeseriesResponse,
  fetchDashboardActivity,
  fetchDashboardNetworkTimeseries,
  fetchParallelWorkStats,
  fetchSummary,
  fetchTimeseries,
  type ParallelWorkStatsResponse,
  type StatsResponse,
  type TimeseriesResponse,
} from "../../lib/api";
import type { DashboardActivityRangeKey } from "./dashboardActivityRange";

export const DASHBOARD_OVERVIEW_SNAPSHOT_SCHEMA_VERSION = 1;
const DASHBOARD_OVERVIEW_SNAPSHOT_DB_NAME = "cvm-dashboard-overview-snapshots";
const DASHBOARD_OVERVIEW_SNAPSHOT_DB_VERSION = 1;
const DASHBOARD_OVERVIEW_SNAPSHOT_STORE_NAME = "overview-snapshots";

export const DASHBOARD_OVERVIEW_SNAPSHOT_RANGES = [
  "today",
  "yesterday",
  "1d",
  "7d",
  "usage",
] as const satisfies readonly DashboardActivityRangeKey[];

export type DashboardOverviewSnapshotRange = (typeof DASHBOARD_OVERVIEW_SNAPSHOT_RANGES)[number];
export type DashboardOverviewSnapshotMode = "live" | "cached-offline" | "not-cached-yet";

export interface DashboardOverviewSnapshotBundle {
  range: DashboardOverviewSnapshotRange;
  dashboardActivity?: DashboardActivityResponse | null;
  summary?: StatsResponse | null;
  timeseries?: TimeseriesResponse | null;
  comparisonSummary?: StatsResponse | null;
  comparisonTimeseries?: TimeseriesResponse | null;
  previous7dSummary?: StatsResponse | null;
  parallelWorkStats?: ParallelWorkStatsResponse | null;
  comparisonParallelWorkStats?: ParallelWorkStatsResponse | null;
  networkTimeseries?: DashboardNetworkTimeseriesResponse | null;
}

export interface DashboardOverviewSnapshotEntry {
  schemaVersion: typeof DASHBOARD_OVERVIEW_SNAPSHOT_SCHEMA_VERSION;
  range: DashboardOverviewSnapshotRange;
  cachedAt: string;
  payload: DashboardOverviewSnapshotBundle;
}

export interface DashboardOverviewSnapshotStatus {
  mode: DashboardOverviewSnapshotMode;
  cachedAt: string | null;
  readyRanges: DashboardOverviewSnapshotRange[];
}

export interface DashboardOverviewSnapshotTestDriver {
  get: (range: DashboardOverviewSnapshotRange) => Promise<unknown>;
  put: (entry: DashboardOverviewSnapshotEntry) => Promise<void>;
  list: () => Promise<unknown[]>;
}

let dashboardOverviewSnapshotTestDriver: DashboardOverviewSnapshotTestDriver | null = null;
let openDatabasePromise: Promise<IDBDatabase | null> | null = null;

function isSnapshotRange(value: unknown): value is DashboardOverviewSnapshotRange {
  return (
    value === "today" ||
    value === "yesterday" ||
    value === "1d" ||
    value === "7d" ||
    value === "usage"
  );
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function sortDashboardOverviewSnapshotRanges(
  ranges: Iterable<DashboardOverviewSnapshotRange>,
): DashboardOverviewSnapshotRange[] {
  const unique = new Set(ranges);
  return DASHBOARD_OVERVIEW_SNAPSHOT_RANGES.filter((range) => unique.has(range));
}

export function installDashboardOverviewSnapshotTestDriver(
  driver: DashboardOverviewSnapshotTestDriver | null,
) {
  dashboardOverviewSnapshotTestDriver = driver;
  openDatabasePromise = null;
}

export function createDashboardOverviewSnapshotEntry(
  range: DashboardOverviewSnapshotRange,
  payload: DashboardOverviewSnapshotBundle,
  cachedAt = new Date().toISOString(),
): DashboardOverviewSnapshotEntry {
  return {
    schemaVersion: DASHBOARD_OVERVIEW_SNAPSHOT_SCHEMA_VERSION,
    range,
    cachedAt,
    payload: {
      ...payload,
      range,
    },
  };
}

export function coerceDashboardOverviewSnapshotEntry(
  raw: unknown,
): DashboardOverviewSnapshotEntry | null {
  if (!isPlainObject(raw)) return null;
  const schemaVersion = raw.schemaVersion;
  const range = raw.range;
  const cachedAt = raw.cachedAt;
  const payload = raw.payload;
  if (schemaVersion !== DASHBOARD_OVERVIEW_SNAPSHOT_SCHEMA_VERSION) return null;
  if (!isSnapshotRange(range)) return null;
  if (typeof cachedAt !== "string" || !Number.isFinite(Date.parse(cachedAt))) return null;
  if (!isPlainObject(payload) || payload.range !== range) return null;
  return {
    schemaVersion: DASHBOARD_OVERVIEW_SNAPSHOT_SCHEMA_VERSION,
    range,
    cachedAt,
    payload: payload as unknown as DashboardOverviewSnapshotBundle,
  };
}

function openDashboardOverviewSnapshotDatabase(): Promise<IDBDatabase | null> {
  if (dashboardOverviewSnapshotTestDriver) {
    return Promise.resolve(null);
  }
  if (typeof indexedDB === "undefined") {
    return Promise.resolve(null);
  }
  if (openDatabasePromise) {
    return openDatabasePromise;
  }
  openDatabasePromise = new Promise((resolve, reject) => {
    const request = indexedDB.open(
      DASHBOARD_OVERVIEW_SNAPSHOT_DB_NAME,
      DASHBOARD_OVERVIEW_SNAPSHOT_DB_VERSION,
    );

    request.onupgradeneeded = () => {
      const database = request.result;
      if (database.objectStoreNames.contains(DASHBOARD_OVERVIEW_SNAPSHOT_STORE_NAME)) {
        database.deleteObjectStore(DASHBOARD_OVERVIEW_SNAPSHOT_STORE_NAME);
      }
      database.createObjectStore(DASHBOARD_OVERVIEW_SNAPSHOT_STORE_NAME, {
        keyPath: "range",
      });
    };

    request.onsuccess = () => {
      const database = request.result;
      database.onversionchange = () => {
        database.close();
        openDatabasePromise = null;
      };
      resolve(database);
    };

    request.onerror = () => {
      reject(request.error ?? new Error("Failed to open dashboard overview snapshot database."));
    };
  });
  return openDatabasePromise;
}

async function withSnapshotStore<T>(
  mode: IDBTransactionMode,
  callback: (store: IDBObjectStore) => Promise<T>,
): Promise<T> {
  const database = await openDashboardOverviewSnapshotDatabase();
  if (!database) {
    throw new Error("Dashboard overview snapshot database is unavailable.");
  }
  const transaction = database.transaction(DASHBOARD_OVERVIEW_SNAPSHOT_STORE_NAME, mode);
  const store = transaction.objectStore(DASHBOARD_OVERVIEW_SNAPSHOT_STORE_NAME);
  const result = await callback(store);
  await new Promise<void>((resolve, reject) => {
    transaction.oncomplete = () => resolve();
    transaction.onerror = () =>
      reject(transaction.error ?? new Error("Dashboard overview snapshot transaction failed."));
    transaction.onabort = () =>
      reject(transaction.error ?? new Error("Dashboard overview snapshot transaction aborted."));
  });
  return result;
}

function requestToPromise<T>(request: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error ?? new Error("IndexedDB request failed."));
  });
}

export async function readDashboardOverviewSnapshotEntry(
  range: DashboardOverviewSnapshotRange,
): Promise<DashboardOverviewSnapshotEntry | null> {
  if (dashboardOverviewSnapshotTestDriver) {
    return coerceDashboardOverviewSnapshotEntry(
      await dashboardOverviewSnapshotTestDriver.get(range),
    );
  }
  if (typeof indexedDB === "undefined") {
    return null;
  }
  const raw = await withSnapshotStore("readonly", (store) => requestToPromise(store.get(range)));
  return coerceDashboardOverviewSnapshotEntry(raw);
}

export async function listDashboardOverviewSnapshotRanges(): Promise<
  DashboardOverviewSnapshotRange[]
> {
  if (dashboardOverviewSnapshotTestDriver) {
    return sortDashboardOverviewSnapshotRanges(
      (await dashboardOverviewSnapshotTestDriver.list())
        .map((raw) => coerceDashboardOverviewSnapshotEntry(raw)?.range ?? null)
        .filter((range): range is DashboardOverviewSnapshotRange => range != null),
    );
  }
  if (typeof indexedDB === "undefined") {
    return [];
  }
  const rawEntries = await withSnapshotStore("readonly", (store) =>
    requestToPromise(store.getAll() as IDBRequest<unknown[]>),
  );
  return sortDashboardOverviewSnapshotRanges(
    rawEntries
      .map((raw) => coerceDashboardOverviewSnapshotEntry(raw)?.range ?? null)
      .filter((range): range is DashboardOverviewSnapshotRange => range != null),
  );
}

export async function writeDashboardOverviewSnapshotEntry(
  entry: DashboardOverviewSnapshotEntry,
): Promise<void> {
  if (dashboardOverviewSnapshotTestDriver) {
    await dashboardOverviewSnapshotTestDriver.put(entry);
    return;
  }
  if (typeof indexedDB === "undefined") {
    return;
  }
  await withSnapshotStore("readwrite", (store) =>
    requestToPromise(store.put(entry)).then(() => undefined),
  );
}

export function isDashboardOverviewSnapshotNetworkError(error: unknown): boolean {
  if (!error) return false;
  if (error instanceof ApiRequestError) {
    return false;
  }
  if (error instanceof DOMException && error.name === "AbortError") {
    return false;
  }
  if (error instanceof TypeError) {
    return true;
  }
  if (!(error instanceof Error)) {
    return false;
  }
  const message = error.message.toLowerCase();
  return (
    message.includes("failed to fetch") ||
    message.includes("network error") ||
    message.includes("networkerror") ||
    message.includes("load failed") ||
    message.includes("timed out") ||
    message.includes("timeout")
  );
}

export function getDashboardOverviewSnapshotPrefetchOrder(
  activeRange: DashboardOverviewSnapshotRange,
): DashboardOverviewSnapshotRange[] {
  return [
    activeRange,
    ...DASHBOARD_OVERVIEW_SNAPSHOT_RANGES.filter((range) => range !== activeRange),
  ];
}

export async function fetchDashboardOverviewSnapshotBundle(
  range: DashboardOverviewSnapshotRange,
  options?: { signal?: AbortSignal },
): Promise<DashboardOverviewSnapshotBundle> {
  const signal = options?.signal;
  switch (range) {
    case "today": {
      const [
        dashboardActivity,
        timeseries,
        comparisonSummary,
        previous7dSummary,
        comparisonTimeseries,
        parallelWorkStats,
        comparisonParallelWorkStats,
        networkTimeseries,
      ] = await Promise.all([
        fetchDashboardActivity("today", {
          includeAccounts: false,
          includeRecent: false,
          signal,
        }),
        fetchTimeseries("today", { bucket: "1m", signal }),
        fetchSummary("yesterday", { signal }),
        fetchSummary("previous7d", { signal }),
        fetchTimeseries("yesterday", { bucket: "1m", signal }),
        fetchParallelWorkStats({ range: "today", bucket: "1m", signal }),
        fetchParallelWorkStats({ range: "yesterday", bucket: "1m", signal }),
        fetchDashboardNetworkTimeseries("today", { signal }),
      ]);
      return {
        range,
        dashboardActivity,
        timeseries,
        comparisonSummary,
        previous7dSummary,
        comparisonTimeseries,
        parallelWorkStats,
        comparisonParallelWorkStats,
        networkTimeseries,
      };
    }
    case "yesterday": {
      const [
        dashboardActivity,
        timeseries,
        previous7dSummary,
        parallelWorkStats,
        networkTimeseries,
      ] = await Promise.all([
        fetchDashboardActivity("yesterday", {
          includeAccounts: false,
          includeRecent: false,
          signal,
        }),
        fetchTimeseries("yesterday", { bucket: "1m", signal }),
        fetchSummary("previous7d", { signal }),
        fetchParallelWorkStats({ range: "yesterday", bucket: "1m", signal }),
        fetchDashboardNetworkTimeseries("yesterday", { signal }),
      ]);
      return {
        range,
        dashboardActivity,
        timeseries,
        previous7dSummary,
        parallelWorkStats,
        networkTimeseries,
      };
    }
    case "1d": {
      const [dashboardActivity, summary, timeseries, networkTimeseries] = await Promise.all([
        fetchDashboardActivity("1d", {
          includeAccounts: false,
          includeRecent: false,
          signal,
        }),
        fetchSummary("1d", { signal }),
        fetchTimeseries("1d", { bucket: "1m", signal }),
        fetchDashboardNetworkTimeseries("1d", { signal }),
      ]);
      return {
        range,
        dashboardActivity,
        summary,
        timeseries,
        networkTimeseries,
      };
    }
    case "7d": {
      const [dashboardActivity, summary, timeseries] = await Promise.all([
        fetchDashboardActivity("7d", {
          includeAccounts: false,
          includeRecent: false,
          signal,
        }),
        fetchSummary("7d", { signal }),
        fetchTimeseries("7d", { bucket: "1h", signal }),
      ]);
      return {
        range,
        dashboardActivity,
        summary,
        timeseries,
      };
    }
    case "usage": {
      const timeseries = await fetchTimeseries("6mo", { bucket: "1d", signal });
      return {
        range,
        timeseries,
      };
    }
  }
}
