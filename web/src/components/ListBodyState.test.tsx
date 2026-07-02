/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { ListBodyState } from "./ListBodyState";

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
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
  vi.clearAllMocks();
});

function render(ui: React.ReactNode) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(ui);
  });
}

describe("ListBodyState", () => {
  it("renders loading skeleton rows inside the body state", () => {
    render(
      <ListBodyState
        variant="loading"
        title="Loading records"
        description="Fetching the first page."
        testId="list-loading"
      />,
    );

    const state = host?.querySelector('[data-testid="list-loading"]');
    expect(state).toBeInstanceOf(HTMLDivElement);
    expect(state?.getAttribute("aria-busy")).toBe("true");
    expect(state?.getAttribute("aria-label")).toBe("Loading records");
    expect(host?.textContent).toContain("Loading records");
    expect(host?.textContent).toContain("Fetching the first page.");
    expect(state?.querySelectorAll(".bg-base-content\\/10")).toHaveLength(12);
  });

  it("renders an initial error with a retry action", () => {
    const onRetry = vi.fn();
    render(
      <ListBodyState
        variant="error"
        title="Failed to load"
        description="Request failed"
        retryLabel="Retry"
        onRetry={onRetry}
        testId="list-error"
      />,
    );

    const state = host?.querySelector('[data-testid="list-error"]');
    const retry = host?.querySelector("button");
    expect(state?.getAttribute("role")).toBe("alert");
    expect(retry).toBeInstanceOf(HTMLButtonElement);
    act(() => {
      retry?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it("renders an empty placeholder after a successful empty response", () => {
    render(
      <ListBodyState
        variant="empty"
        title="No results"
        description="Adjust filters or create a record."
        testId="list-empty"
      />,
    );

    const state = host?.querySelector('[data-testid="list-empty"]');
    expect(state?.getAttribute("aria-busy")).toBeNull();
    expect(host?.textContent).toContain("No results");
    expect(host?.textContent).toContain("Adjust filters or create a record.");
  });
});
