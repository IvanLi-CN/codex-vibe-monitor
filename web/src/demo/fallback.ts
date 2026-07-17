const DEMO_FETCH_FALLBACK_KEY = Symbol.for("codex-vibe-monitor.demo.fetch-fallback");

type DemoFallbackWindow = Window & {
  [DEMO_FETCH_FALLBACK_KEY]?: boolean;
};

function isDemoApiRequest(request: Request) {
  if (typeof window === "undefined") return false;
  const url = new URL(request.url, window.location.href);
  return url.origin === window.location.origin && url.pathname.includes("/api/");
}

export function installDemoFetchFallback(
  handleDemoRequest: (request: Request) => Promise<Response>,
) {
  if (typeof window === "undefined") return;

  const demoWindow = window as DemoFallbackWindow;
  if (demoWindow[DEMO_FETCH_FALLBACK_KEY]) return;

  const originalFetch = window.fetch.bind(window);
  window.fetch = async (input: RequestInfo | URL, init?: RequestInit) => {
    const request = input instanceof Request && init == null ? input : new Request(input, init);
    if (isDemoApiRequest(request)) {
      return handleDemoRequest(request);
    }
    return originalFetch(input, init);
  };
  demoWindow[DEMO_FETCH_FALLBACK_KEY] = true;
}
