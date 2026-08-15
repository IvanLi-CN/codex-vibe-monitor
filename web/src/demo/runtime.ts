export type DemoScene =
  | "operational"
  | "attention"
  | "empty"
  | "progressive-loading"
  | "network-failure"
  | "runtime-pressure-healthy"
  | "runtime-pressure-deferred"
  | "runtime-pressure-degraded"
  | "runtime-pressure-accounting-error";
export type DemoTheme = "light" | "dark";
export type DemoViewport = "default" | "mobile390" | "mobile393";

const RUNTIME_VALUES = new Set(["live", "demo"]);
const SCENE_VALUES = new Set<DemoScene>([
  "operational",
  "attention",
  "empty",
  "progressive-loading",
  "network-failure",
  "runtime-pressure-healthy",
  "runtime-pressure-deferred",
  "runtime-pressure-degraded",
  "runtime-pressure-accounting-error",
]);
const THEME_VALUES = new Set<DemoTheme>(["light", "dark"]);
const VIEWPORT_VALUES = new Set<DemoViewport>(["default", "mobile390", "mobile393"]);

function hashSearchFromLocation(
  location: Location | undefined = typeof window === "undefined" ? undefined : window.location,
) {
  if (!location?.hash.includes("?")) return "";
  return location.hash.slice(location.hash.indexOf("?") + 1);
}

export function demoSearchParamsFromLocation(
  location: Location | undefined = typeof window === "undefined" ? undefined : window.location,
): URLSearchParams {
  if (!location) return new URLSearchParams();
  return new URLSearchParams(hashSearchFromLocation(location) || location.search);
}

export function appRuntime(): "live" | "demo" {
  const value = import.meta.env.VITE_APP_RUNTIME ?? "live";
  if (!RUNTIME_VALUES.has(value)) {
    throw new Error(`Unsupported VITE_APP_RUNTIME: ${value}`);
  }
  return value as "live" | "demo";
}

export function isDemoRuntime(): boolean {
  return appRuntime() === "demo";
}

export function sceneFromLocation(
  location: Location | undefined = typeof window === "undefined" ? undefined : window.location,
): DemoScene {
  if (!location) return "operational";
  const scene = demoSearchParamsFromLocation(location).get("demoScene");
  return scene && SCENE_VALUES.has(scene as DemoScene) ? (scene as DemoScene) : "operational";
}

export function themeFromLocation(
  location: Location | undefined = typeof window === "undefined" ? undefined : window.location,
): DemoTheme {
  if (!location) return "light";
  const theme = demoSearchParamsFromLocation(location).get("demoTheme");
  return theme && THEME_VALUES.has(theme as DemoTheme) ? (theme as DemoTheme) : "light";
}

export function viewportFromLocation(
  location: Location | undefined = typeof window === "undefined" ? undefined : window.location,
): DemoViewport {
  if (!location) return "default";
  const viewport = demoSearchParamsFromLocation(location).get("demoViewport");
  return viewport && VIEWPORT_VALUES.has(viewport as DemoViewport)
    ? (viewport as DemoViewport)
    : "default";
}

export function isEmbeddedDemoViewport(
  location: Location | undefined = typeof window === "undefined" ? undefined : window.location,
) {
  if (!location) return false;
  return demoSearchParamsFromLocation(location).get("demoEmbed") === "1";
}

export function shouldStartDemoServiceWorker(
  location: Location | undefined = typeof window === "undefined" ? undefined : window.location,
): boolean {
  return !isEmbeddedDemoViewport(location) && viewportFromLocation(location) === "default";
}

export async function initializeDemoRuntime(): Promise<void> {
  if (!isDemoRuntime()) return;

  const [
    { demoModel },
    { installDemoFetchFallback },
    { installDemoEventSource },
    { handleDemoRequest },
  ] = await Promise.all([
    import("./model"),
    import("./fallback"),
    import("./event-source"),
    import("./handlers"),
  ]);
  demoModel.setScene(sceneFromLocation());
  installDemoFetchFallback(handleDemoRequest);
  installDemoEventSource();
  if (!shouldStartDemoServiceWorker()) return;

  const [{ isCommonAssetRequest }, { worker }] = await Promise.all([
    import("msw"),
    import("./browser"),
  ]);
  await worker.start({
    serviceWorker: {
      url: `${import.meta.env.BASE_URL}mockServiceWorker.js`,
    },
    onUnhandledRequest(request, print) {
      if (!isCommonAssetRequest(request)) print.error();
    },
  });
}
