/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { I18nProvider } from "../../i18n";
import type { ErrorDistributionItem } from "../../lib/api";
import { ThemeProvider } from "../../theme";
import { ErrorReasonDistribution } from "./ErrorReasonDistribution";

const oldItems: ErrorDistributionItem[] = [{ reason: "The previous scope response", count: 4 }];

let host: HTMLDivElement | null = null;
let root: Root | null = null;

beforeAll(() => {
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
    configurable: true,
    writable: true,
    value: true,
  });
});

afterEach(() => {
  act(() => root?.unmount());
  host?.remove();
  host = null;
  root = null;
});

function renderDistribution({
  isLoading = false,
  error = null,
  items = [],
}: {
  isLoading?: boolean;
  error?: string | null;
  items?: ErrorDistributionItem[];
} = {}) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(
      <ThemeProvider>
        <I18nProvider initialLocale="en" persistLocale={false}>
          <ErrorReasonDistribution
            items={items}
            isLoading={isLoading}
            error={error}
            scope="service"
            onScopeChange={() => undefined}
          />
        </I18nProvider>
      </ThemeProvider>,
    );
  });
  return host;
}

describe("ErrorReasonDistribution", () => {
  it("keeps the scope selector visible and hides stale rows while loading", () => {
    const view = renderDistribution({ isLoading: true, items: oldItems });

    expect(view.querySelector('[data-testid="stats-error-scope-select-trigger"]')).toBeTruthy();
    expect(view.querySelector('[data-testid="error-distribution-loading"]')).toBeTruthy();
    expect(view.querySelectorAll('[data-testid="error-reason-row"]')).toHaveLength(0);
    expect(view.textContent).not.toContain("The previous scope response");
  });

  it("keeps the scope selector available for empty and error responses", () => {
    const view = renderDistribution({ error: "Unable to load error reasons." });

    expect(view.querySelector('[data-testid="stats-error-scope-select-trigger"]')).toBeTruthy();
    expect(view.querySelector('[role="status"]')?.textContent).toContain(
      "Unable to load error reasons.",
    );

    act(() => {
      root?.render(
        <ThemeProvider>
          <I18nProvider initialLocale="en" persistLocale={false}>
            <ErrorReasonDistribution
              items={[]}
              isLoading={false}
              error={null}
              scope="service"
              onScopeChange={() => undefined}
            />
          </I18nProvider>
        </ThemeProvider>,
      );
    });

    expect(view.querySelector('[data-testid="stats-error-scope-select-trigger"]')).toBeTruthy();
    expect(view.querySelector('[role="status"]')).toBeTruthy();
  });
});
