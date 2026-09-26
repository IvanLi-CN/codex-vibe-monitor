/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { I18nProvider } from "../../i18n";
import { ModelBreakdownModeToggle } from "./DashboardModelBreakdownControls";
import { DASHBOARD_MODEL_BREAKDOWN_MODE_STORAGE_KEY } from "./dashboardModelBreakdown";

let host: HTMLDivElement | null = null;
let root: Root | null = null;

beforeAll(() => {
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
    configurable: true,
    writable: true,
    value: true,
  });
});

beforeEach(() => {
  window.localStorage.removeItem(DASHBOARD_MODEL_BREAKDOWN_MODE_STORAGE_KEY);
});

afterEach(() => {
  act(() => root?.unmount());
  host?.remove();
  host = null;
  root = null;
  window.localStorage.removeItem(DASHBOARD_MODEL_BREAKDOWN_MODE_STORAGE_KEY);
});

describe("ModelBreakdownModeToggle", () => {
  it("synchronizes the mode across every mounted toggle", async () => {
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);

    await act(async () => {
      root?.render(
        <I18nProvider>
          <ModelBreakdownModeToggle />
          <ModelBreakdownModeToggle />
        </I18nProvider>,
      );
      await Promise.resolve();
    });

    const simpleButtons = host.querySelectorAll<HTMLButtonElement>(
      '[data-testid="dashboard-model-breakdown-mode-simple"]',
    );
    expect(simpleButtons).toHaveLength(2);

    await act(async () => {
      simpleButtons[0]?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await Promise.resolve();
    });

    expect(
      Array.from(
        host.querySelectorAll('[data-testid="dashboard-model-breakdown-mode-simple"]'),
      ).every((button) => button.getAttribute("aria-selected") === "true"),
    ).toBe(true);
    expect(window.localStorage.getItem(DASHBOARD_MODEL_BREAKDOWN_MODE_STORAGE_KEY)).toBe("simple");
  });
});
