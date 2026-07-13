export type DemoScene = "operational" | "attention" | "empty" | "network-failure";
export type DemoTheme = "light" | "dark";

const RUNTIME_VALUES = new Set(["live", "demo"]);
const SCENE_VALUES = new Set<DemoScene>(["operational", "attention", "empty", "network-failure"]);
const THEME_VALUES = new Set<DemoTheme>(["light", "dark"]);

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
  const hashSearch = location.hash.includes("?")
    ? location.hash.slice(location.hash.indexOf("?") + 1)
    : "";
  const scene = new URLSearchParams(hashSearch).get("demoScene");
  return scene && SCENE_VALUES.has(scene as DemoScene) ? (scene as DemoScene) : "operational";
}

export function themeFromLocation(
  location: Location | undefined = typeof window === "undefined" ? undefined : window.location,
): DemoTheme {
  if (!location) return "light";
  const hashSearch = location.hash.includes("?")
    ? location.hash.slice(location.hash.indexOf("?") + 1)
    : "";
  const theme = new URLSearchParams(hashSearch).get("demoTheme");
  return theme && THEME_VALUES.has(theme as DemoTheme) ? (theme as DemoTheme) : "light";
}

export async function initializeDemoRuntime(): Promise<void> {
  if (!isDemoRuntime()) return;

  const [{ isCommonAssetRequest }, { demoModel }, { worker }] = await Promise.all([
    import("msw"),
    import("./model"),
    import("./browser"),
  ]);
  demoModel.setScene(sceneFromLocation());
  await worker.start({
    serviceWorker: {
      url: `${import.meta.env.BASE_URL}mockServiceWorker.js`,
    },
    onUnhandledRequest(request, print) {
      if (!isCommonAssetRequest(request)) print.error();
    },
  });
}
