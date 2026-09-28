import { useEffect, useRef } from "react";
import { type BrowserPerformanceEvent, postBrowserPerformanceTelemetry } from "./api";

type PageFamily = BrowserPerformanceEvent["page"];

function pageFamily(pathname: string): PageFamily | null {
  if (pathname === "/dashboard" || pathname.startsWith("/dashboard/")) return "dashboard";
  if (pathname === "/records" || pathname.startsWith("/records/")) return "records";
  if (pathname === "/system" || pathname.startsWith("/system/")) return "system";
  return null;
}

function deviceClass(): BrowserPerformanceEvent["device"] {
  if (typeof window !== "undefined" && window.matchMedia?.("(max-width: 768px)").matches) {
    return "mobile";
  }
  return "desktop";
}

function supportedRuntime() {
  return (
    typeof window !== "undefined" &&
    typeof performance !== "undefined" &&
    typeof fetch === "function"
  );
}

export function BrowserPerformanceTelemetry({ pathname }: { pathname: string }) {
  const family = pageFamily(pathname);
  const familyRef = useRef<PageFamily | null>(family);
  const eventsRef = useRef<BrowserPerformanceEvent[]>([]);
  const lastSentAtRef = useRef(0);

  useEffect(() => {
    familyRef.current = family;
  }, [family]);

  useEffect(() => {
    if (!supportedRuntime() || !family) return;
    const startedAt = performance.now();
    const add = (metric: BrowserPerformanceEvent["metric"], value: number) => {
      if (!Number.isFinite(value) || value < 0 || eventsRef.current.length >= 8) return;
      eventsRef.current.push({ page: family, device: deviceClass(), metric, value });
    };
    const frame = window.requestAnimationFrame(() => {
      add("data_ready_ms", performance.now() - startedAt);
      window.requestAnimationFrame(() => add("update_to_paint_ms", performance.now() - startedAt));
    });
    return () => window.cancelAnimationFrame(frame);
  }, [family]);

  useEffect(() => {
    const initialFamily = familyRef.current;
    if (!supportedRuntime() || !initialFamily) return;
    if (typeof PerformanceObserver === "undefined") {
      if (eventsRef.current.length < 8) {
        eventsRef.current.push({
          page: initialFamily,
          device: deviceClass(),
          metric: "unsupported_count",
          value: 1,
        });
      }
      return;
    }
    let observer: PerformanceObserver | null = null;
    try {
      observer = new PerformanceObserver((list) => {
        const currentFamily = familyRef.current;
        if (!currentFamily) return;
        for (const entry of list.getEntries()) {
          if (entry.duration <= 0 || eventsRef.current.length >= 8) continue;
          eventsRef.current.push({
            page: currentFamily,
            device: deviceClass(),
            metric: "long_task_ms",
            value: entry.duration,
          });
          if (eventsRef.current.length < 8) {
            eventsRef.current.push({
              page: currentFamily,
              device: deviceClass(),
              metric: "long_task_count",
              value: 1,
            });
          }
        }
      });
      observer.observe({ type: "longtask", buffered: true });
    } catch {
      observer?.disconnect();
      observer = null;
      const currentFamily = familyRef.current;
      if (currentFamily && eventsRef.current.length < 8) {
        eventsRef.current.push({
          page: currentFamily,
          device: deviceClass(),
          metric: "unsupported_count",
          value: 1,
        });
      }
    }
    return () => observer?.disconnect();
  }, []);

  useEffect(() => {
    if (!supportedRuntime() || typeof PerformanceObserver === "undefined") return;
    let observer: PerformanceObserver | null = null;
    try {
      observer = new PerformanceObserver((list) => {
        const currentFamily = familyRef.current;
        if (!currentFamily) return;
        for (const entry of list.getEntries()) {
          if (eventsRef.current.length >= 8 || entry.duration < 0) break;
          let pathname: string;
          try {
            pathname = new URL(entry.name, window.location.href).pathname;
          } catch {
            continue;
          }
          if (pathname === "/events") {
            eventsRef.current.push({
              page: currentFamily,
              device: deviceClass(),
              metric: "sse_duration_ms",
              value: entry.duration,
            });
            if (eventsRef.current.length < 8) {
              eventsRef.current.push({
                page: currentFamily,
                device: deviceClass(),
                metric: "sse_disconnect_count",
                value: 1,
              });
            }
          } else if (
            pathname.startsWith("/api/") &&
            !pathname.startsWith("/api/system/performance")
          ) {
            eventsRef.current.push({
              page: currentFamily,
              device: deviceClass(),
              metric: "api_request_duration_ms",
              value: entry.duration,
            });
            if (eventsRef.current.length < 8) {
              eventsRef.current.push({
                page: currentFamily,
                device: deviceClass(),
                metric: "api_request_count",
                value: 1,
              });
            }
          }
        }
      });
      observer.observe({ type: "resource", buffered: true });
    } catch {
      observer?.disconnect();
      observer = null;
      const currentFamily = familyRef.current;
      if (currentFamily && eventsRef.current.length < 8) {
        eventsRef.current.push({
          page: currentFamily,
          device: deviceClass(),
          metric: "unsupported_count",
          value: 1,
        });
      }
    }
    return () => observer?.disconnect();
  }, []);

  useEffect(() => {
    if (!supportedRuntime()) return;
    const flush = async () => {
      const now = Date.now();
      if (
        now - lastSentAtRef.current < 60_000 ||
        eventsRef.current.length === 0 ||
        navigator.onLine === false
      )
        return;
      const events = eventsRef.current.splice(0, 8);
      try {
        await postBrowserPerformanceTelemetry(events);
        lastSentAtRef.current = now;
      } catch {
        eventsRef.current.unshift(...events.slice(0, 8 - eventsRef.current.length));
      }
    };
    const timer = window.setInterval(() => void flush(), 60_000);
    const onVisibilityChange = () => {
      if (document.visibilityState !== "hidden") return;
      const currentFamily = familyRef.current;
      if (currentFamily && eventsRef.current.length < 8) {
        eventsRef.current.push({
          page: currentFamily,
          device: deviceClass(),
          metric: "visibility_hidden_count",
          value: 1,
        });
      }
      void flush();
    };
    document.addEventListener("visibilitychange", onVisibilityChange);
    return () => {
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisibilityChange);
    };
  }, []);

  return null;
}
