/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import usePwaRuntime from "./usePwaRuntime";

const registerSwMock = vi.hoisted(() => vi.fn());

vi.mock("virtual:pwa-register", () => ({
  registerSW: registerSwMock,
}));

let host: HTMLDivElement | null = null;
let root: Root | null = null;
let standaloneDisplay = false;

function installBrowserMocks() {
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    value: (query: string) => ({
      matches:
        standaloneDisplay &&
        (query === "(display-mode: standalone)" ||
          query === "(display-mode: window-controls-overlay)"),
      media: query,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      addListener: vi.fn(),
      removeListener: vi.fn(),
      dispatchEvent: vi.fn(),
      onchange: null,
    }),
  });

  Object.defineProperty(window.navigator, "serviceWorker", {
    configurable: true,
    value: {
      controller: standaloneDisplay ? {} : null,
    },
  });

  Object.defineProperty(window.navigator, "onLine", {
    configurable: true,
    value: true,
  });

  Object.defineProperty(window.navigator, "userAgent", {
    configurable: true,
    value:
      "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15",
  });
  Object.defineProperty(window.navigator, "platform", {
    configurable: true,
    value: "MacIntel",
  });
  Object.defineProperty(window.navigator, "maxTouchPoints", {
    configurable: true,
    value: 0,
  });
}

function Harness() {
  const runtime = usePwaRuntime();

  return (
    <div>
      <span data-testid="install-mode">{runtime.installMode}</span>
      <span data-testid="install-supported">{String(runtime.installSupported)}</span>
      <span data-testid="is-offline">{String(runtime.isOffline)}</span>
      <span data-testid="shell-ready">{String(runtime.shellReady)}</span>
      <span data-testid="update-visible">{String(runtime.update.visible)}</span>
      <span data-testid="available-version">{runtime.update.availableVersion ?? "none"}</span>
      <button
        type="button"
        data-testid="prompt-install"
        onClick={() => {
          void runtime.promptInstall();
        }}
      />
      <button
        type="button"
        data-testid="apply-update"
        onClick={() => {
          void runtime.applyUpdate();
        }}
      />
      <button type="button" data-testid="dismiss-update" onClick={runtime.dismissUpdate} />
    </div>
  );
}

function render() {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(<Harness />);
  });
}

beforeAll(() => {
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
    configurable: true,
    writable: true,
    value: true,
  });
});

beforeEach(() => {
  standaloneDisplay = false;
  installBrowserMocks();
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => new Response(JSON.stringify({ version: "0.2.1" }), { status: 200 })),
  );
  registerSwMock.mockReset();
});

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
  vi.unstubAllGlobals();
});

describe("usePwaRuntime", () => {
  it("captures beforeinstallprompt and transitions into installed mode after acceptance", async () => {
    const updateServiceWorker = vi.fn();
    registerSwMock.mockImplementation(() => updateServiceWorker);

    render();

    const promptMock = vi.fn(async () => {
      window.dispatchEvent(new Event("appinstalled"));
    });
    const installEvent = new Event("beforeinstallprompt") as Event & {
      prompt: () => Promise<void>;
      userChoice: Promise<{ outcome: "accepted"; platform: "web" }>;
    };
    installEvent.preventDefault = vi.fn();
    installEvent.prompt = promptMock;
    installEvent.userChoice = Promise.resolve({ outcome: "accepted", platform: "web" });

    await act(async () => {
      window.dispatchEvent(installEvent);
      await Promise.resolve();
    });

    expect(host?.querySelector('[data-testid="install-mode"]')?.textContent).toBe("prompt");

    await act(async () => {
      (host?.querySelector('[data-testid="prompt-install"]') as HTMLButtonElement | null)?.click();
      await Promise.resolve();
    });

    expect(promptMock).toHaveBeenCalledTimes(1);
    expect(host?.querySelector('[data-testid="install-mode"]')?.textContent).toBe("installed");
  });

  it("exposes manual iOS Safari guidance when no native install prompt is available", async () => {
    Object.defineProperty(window.navigator, "userAgent", {
      configurable: true,
      value:
        "Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.1 Mobile/15E148 Safari/604.1",
    });
    Object.defineProperty(window.navigator, "platform", {
      configurable: true,
      value: "iPhone",
    });
    Object.defineProperty(window.navigator, "maxTouchPoints", {
      configurable: true,
      value: 5,
    });
    registerSwMock.mockImplementation(() => vi.fn());

    render();

    await act(async () => {
      await Promise.resolve();
    });

    expect(host?.querySelector('[data-testid="install-mode"]')?.textContent).toBe("manual-ios");
    expect(host?.querySelector('[data-testid="install-supported"]')?.textContent).toBe("true");
  });

  it("surfaces offline shell readiness, browser offline state, and prompt-style updates", async () => {
    let registerOptions:
      | {
          onOfflineReady?: () => void;
          onNeedRefresh?: () => void;
        }
      | undefined;
    const updateServiceWorker = vi.fn();

    registerSwMock.mockImplementation((options) => {
      registerOptions = options;
      return updateServiceWorker;
    });

    render();

    await act(async () => {
      registerOptions?.onOfflineReady?.();
      await Promise.resolve();
    });

    expect(host?.querySelector('[data-testid="shell-ready"]')?.textContent).toBe("true");

    await act(async () => {
      await registerOptions?.onNeedRefresh?.();
      await Promise.resolve();
    });

    expect(host?.querySelector('[data-testid="update-visible"]')?.textContent).toBe("true");
    expect(host?.querySelector('[data-testid="available-version"]')?.textContent).toBe("v0.2.1");

    await act(async () => {
      (host?.querySelector('[data-testid="apply-update"]') as HTMLButtonElement | null)?.click();
      await Promise.resolve();
    });

    expect(updateServiceWorker).toHaveBeenCalledWith(true);

    await act(async () => {
      Object.defineProperty(window.navigator, "onLine", {
        configurable: true,
        value: false,
      });
      window.dispatchEvent(new Event("offline"));
      await Promise.resolve();
    });

    expect(host?.querySelector('[data-testid="is-offline"]')?.textContent).toBe("true");
  });
});
