import { afterEach, describe, expect, it, vi } from "vitest";
import {
  appRuntime,
  isEmbeddedDemoViewport,
  sceneFromLocation,
  themeFromLocation,
  viewportFromLocation,
} from "./runtime";

afterEach(() => {
  vi.unstubAllEnvs();
});

describe("demo runtime selection", () => {
  it("allows only live and demo runtimes", () => {
    vi.stubEnv("VITE_APP_RUNTIME", "demo");
    expect(appRuntime()).toBe("demo");

    vi.stubEnv("VITE_APP_RUNTIME", "unsupported");
    expect(() => appRuntime()).toThrow("Unsupported VITE_APP_RUNTIME");
  });

  it("uses only hash query state for shareable scene and theme", () => {
    const location = new URL(
      "https://demo.invalid/#/records?demoScene=network-failure&demoTheme=dark",
    ) as unknown as Location;

    expect(sceneFromLocation(location)).toBe("network-failure");
    expect(themeFromLocation(location)).toBe("dark");
  });

  it("accepts the progressive account loading scene", () => {
    const location = new URL(
      "https://demo.invalid/#/dashboard?demoScene=progressive-loading",
    ) as unknown as Location;

    expect(sceneFromLocation(location)).toBe("progressive-loading");
  });

  it("parses the mobile viewport wrapper state from hash query params", () => {
    const outerLocation = new URL(
      "https://demo.invalid/#/dashboard/invocations/demo-invocation-9002?demoScene=attention&demoViewport=mobile390",
    ) as unknown as Location;
    const embeddedLocation = new URL(
      "https://demo.invalid/#/dashboard/invocations/demo-invocation-9002?demoScene=attention&demoEmbed=1",
    ) as unknown as Location;

    expect(viewportFromLocation(outerLocation)).toBe("mobile390");
    expect(isEmbeddedDemoViewport(outerLocation)).toBe(false);
    expect(viewportFromLocation(embeddedLocation)).toBe("default");
    expect(isEmbeddedDemoViewport(embeddedLocation)).toBe(true);
  });
});
