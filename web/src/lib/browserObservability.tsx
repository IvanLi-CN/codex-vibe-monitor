import { useEffect, useRef, useSyncExternalStore } from "react";
import { fetchObservabilityCapabilities, postBrowserObservations } from "./api";
import {
  browserObservationEnabled,
  configureBrowserObservation,
  flushBrowserObservations,
  type ObservationPage,
  observeBrowser,
  subscribeBrowserObservation,
} from "./observation-events";

function family(path: string): ObservationPage | null {
  for (const page of ["dashboard", "records", "system"] as const)
    if (path === `/${page}` || path.startsWith(`/${page}/`)) return page;
  return null;
}
export function usePageObservation(page: ObservationPage, data: unknown): void {
  const active = useSyncExternalStore(
    subscribeBrowserObservation,
    browserObservationEnabled,
    () => false,
  );
  const started = useRef<number | null>(null);
  const readyAt = useRef<number | null>(null);
  const ready = useRef(false);
  const pendingPaint = useRef<number | null>(null);
  if (started.current === null && typeof performance !== "undefined")
    started.current = performance.now();
  useEffect(() => {
    if (data == null || started.current === null) return;
    const updatedAt = performance.now();
    if (readyAt.current === null) readyAt.current = updatedAt;
    if (typeof window.requestAnimationFrame !== "function") {
      observeBrowser("unsupported", 0, page);
      return;
    }
    // The second animation frame observes a paint opportunity after the committed data update.
    let second = 0;
    const first = window.requestAnimationFrame(() => {
      second = window.requestAnimationFrame(() => {
        const duration = (performance.now() - updatedAt) / 1000;
        if (browserObservationEnabled()) observeBrowser("update_to_paint", duration, page);
        else pendingPaint.current = duration;
      });
    });
    return (): void => {
      window.cancelAnimationFrame(first);
      window.cancelAnimationFrame(second);
    };
  }, [data, page]);
  useEffect(() => {
    if (!active || data == null || readyAt.current === null || started.current === null) return;
    if (!ready.current) {
      observeBrowser("data_ready", (readyAt.current - started.current) / 1000, page);
      ready.current = true;
    }
    if (pendingPaint.current !== null) {
      observeBrowser("update_to_paint", pendingPaint.current, page);
      pendingPaint.current = null;
    }
    if (typeof window.requestAnimationFrame !== "function") observeBrowser("unsupported", 0, page);
  }, [active, data, page]);
}
export function BrowserObservability({ pathname }: { pathname: string }): null {
  const active = useSyncExternalStore(
    subscribeBrowserObservation,
    browserObservationEnabled,
    () => false,
  );
  const activeRef = useRef(false);
  const page = family(pathname);
  const currentPage = useRef(page);
  currentPage.current = page;
  useEffect(() => {
    let mounted = true;
    void fetchObservabilityCapabilities()
      .then((value) => {
        if (mounted) {
          activeRef.current = value.enabled;
          configureBrowserObservation(currentPage.current, value.enabled);
        }
      })
      .catch(() => {});
    return (): void => {
      mounted = false;
      configureBrowserObservation(null, false);
    };
  }, []);
  useEffect(() => {
    configureBrowserObservation(page, activeRef.current);
  }, [page]);
  useEffect(() => {
    if (!active) return;
    const observers: PerformanceObserver[] = [];
    const watch = (type: string, consume: (entry: PerformanceEntry) => void): void => {
      try {
        if (!PerformanceObserver.supportedEntryTypes.includes(type)) {
          observeBrowser("unsupported");
          return;
        }
        const observer = new PerformanceObserver((list) => {
          for (const entry of list.getEntries()) consume(entry);
        });
        observer.observe({ type, buffered: false });
        observers.push(observer);
      } catch {
        observeBrowser("unsupported");
      }
    };
    if (typeof PerformanceObserver === "undefined") observeBrowser("unsupported");
    else {
      watch("longtask", (entry) => observeBrowser("long_task", entry.duration / 1000));
      watch("resource", (entry) => {
        try {
          const url = new URL(entry.name, window.location.href);
          if (
            url.origin === window.location.origin &&
            url.pathname.startsWith("/api/") &&
            !url.pathname.startsWith("/api/system/observability") &&
            !url.pathname.startsWith("/api/system/performance") &&
            (entry as PerformanceResourceTiming).initiatorType !== "other"
          )
            observeBrowser("api_request", entry.duration / 1000);
        } catch {
          /* Invalid resource names are not measurements. */
        }
      });
    }
    const flush = (): void => {
      void flushBrowserObservations(postBrowserObservations);
    };
    const timer = window.setInterval(flush, 60_000);
    const visibility = (): void => {
      if (document.visibilityState === "hidden") {
        observeBrowser("hidden");
        flush();
      }
    };
    document.addEventListener("visibilitychange", visibility);
    return (): void => {
      for (const observer of observers) observer.disconnect();
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", visibility);
    };
  }, [active]);
  return null;
}
