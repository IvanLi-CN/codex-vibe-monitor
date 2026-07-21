export type DemoScene =
  | "operational"
  | "attention"
  | "empty"
  | "progressive-loading"
  | "network-failure";
export type DemoTheme = "light" | "dark";
export type DemoViewport = "default" | "mobile390";

const RUNTIME_VALUES = new Set(["live", "demo"]);
const SCENE_VALUES = new Set<DemoScene>([
  "operational",
  "attention",
  "empty",
  "progressive-loading",
  "network-failure",
]);
const THEME_VALUES = new Set<DemoTheme>(["light", "dark"]);
const VIEWPORT_VALUES = new Set<DemoViewport>(["default", "mobile390"]);

function hashSearchFromLocation(
  location: Location | undefined = typeof window === "undefined" ? undefined : window.location,
) {
  if (!location?.hash.includes("?")) return "";
  return location.hash.slice(location.hash.indexOf("?") + 1);
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
  const hashSearch = hashSearchFromLocation(location);
  const scene = new URLSearchParams(hashSearch).get("demoScene");
  return scene && SCENE_VALUES.has(scene as DemoScene) ? (scene as DemoScene) : "operational";
}

export function themeFromLocation(
  location: Location | undefined = typeof window === "undefined" ? undefined : window.location,
): DemoTheme {
  if (!location) return "light";
  const hashSearch = hashSearchFromLocation(location);
  const theme = new URLSearchParams(hashSearch).get("demoTheme");
  return theme && THEME_VALUES.has(theme as DemoTheme) ? (theme as DemoTheme) : "light";
}

export function viewportFromLocation(
  location: Location | undefined = typeof window === "undefined" ? undefined : window.location,
): DemoViewport {
  if (!location) return "default";
  const hashSearch = hashSearchFromLocation(location);
  const viewport = new URLSearchParams(hashSearch).get("demoViewport");
  return viewport && VIEWPORT_VALUES.has(viewport as DemoViewport)
    ? (viewport as DemoViewport)
    : "default";
}

export function isEmbeddedDemoViewport(
  location: Location | undefined = typeof window === "undefined" ? undefined : window.location,
) {
  if (!location) return false;
  return new URLSearchParams(hashSearchFromLocation(location)).get("demoEmbed") === "1";
}

export async function initializeDemoRuntime(): Promise<void> {
  if (!isDemoRuntime()) return;

  const [
    { isCommonAssetRequest },
    { demoModel },
    { worker },
    { installDemoFetchFallback },
    { handleDemoRequest },
  ] = await Promise.all([
    import("msw"),
    import("./model"),
    import("./browser"),
    import("./fallback"),
    import("./handlers"),
  ]);
  demoModel.setScene(sceneFromLocation());
  installDemoFetchFallback(handleDemoRequest);
  await worker.start({
    serviceWorker: {
      url: `${import.meta.env.BASE_URL}mockServiceWorker.js`,
    },
    onUnhandledRequest(request, print) {
      if (!isCommonAssetRequest(request)) print.error();
    },
  });
}
