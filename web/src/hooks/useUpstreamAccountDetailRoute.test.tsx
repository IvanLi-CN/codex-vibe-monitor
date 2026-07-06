/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter, Route, Routes, useLocation } from "react-router-dom";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { useUpstreamAccountDetailRoute } from "./useUpstreamAccountDetailRoute";

function RouteProbe() {
  const location = useLocation();
  const {
    upstreamAccountId,
    upstreamAccountTab,
    openUpstreamAccount,
    closeUpstreamAccount,
  } = useUpstreamAccountDetailRoute();

  return (
    <div>
      <div data-testid="route-search">{location.search}</div>
      <div data-testid="route-account-id">{String(upstreamAccountId)}</div>
      <div data-testid="route-account-tab">{upstreamAccountTab}</div>
      <button
        type="button"
        data-testid="open-routing"
        onClick={() => openUpstreamAccount(42, { tab: "routing" })}
      >
        open routing
      </button>
      <button
        type="button"
        data-testid="open-overview"
        onClick={() => openUpstreamAccount(77)}
      >
        open overview
      </button>
      <button
        type="button"
        data-testid="open-health-events"
        onClick={() => openUpstreamAccount(42, { tab: "healthEvents" })}
      >
        open health events
      </button>
      <button
        type="button"
        data-testid="close-account"
        onClick={() => closeUpstreamAccount()}
      >
        close
      </button>
    </div>
  );
}

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
});

function render(initialEntry = "/dashboard") {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(
      <MemoryRouter initialEntries={[initialEntry]}>
        <Routes>
          <Route path="/dashboard" element={<RouteProbe />} />
        </Routes>
      </MemoryRouter>,
    );
  });
}

describe("useUpstreamAccountDetailRoute", () => {
  it("parses the upstream account id and routing tab from the URL", () => {
    render("/dashboard?upstreamAccountId=42&upstreamAccountTab=routing");

    expect(
      host?.querySelector('[data-testid="route-account-id"]')?.textContent,
    ).toBe("42");
    expect(
      host?.querySelector('[data-testid="route-account-tab"]')?.textContent,
    ).toBe("routing");
  });

  it("parses the health events tab from the URL", () => {
    render("/dashboard?upstreamAccountId=42&upstreamAccountTab=healthEvents");

    expect(
      host?.querySelector('[data-testid="route-account-id"]')?.textContent,
    ).toBe("42");
    expect(
      host?.querySelector('[data-testid="route-account-tab"]')?.textContent,
    ).toBe("healthEvents");
  });

  it("falls back to overview when the query tab is invalid", () => {
    render("/dashboard?upstreamAccountId=42&upstreamAccountTab=records");

    expect(
      host?.querySelector('[data-testid="route-account-id"]')?.textContent,
    ).toBe("42");
    expect(
      host?.querySelector('[data-testid="route-account-tab"]')?.textContent,
    ).toBe("overview");
  });

  it("writes and clears the routing tab query parameters", () => {
    render("/dashboard");

    const openRoutingButton = host?.querySelector(
      '[data-testid="open-routing"]',
    );
    if (!(openRoutingButton instanceof HTMLButtonElement)) {
      throw new Error("missing open routing button");
    }

    act(() => {
      openRoutingButton.click();
    });

    expect(
      host?.querySelector('[data-testid="route-search"]')?.textContent,
    ).toBe("?upstreamAccountId=42&upstreamAccountTab=routing");
    expect(
      host?.querySelector('[data-testid="route-account-tab"]')?.textContent,
    ).toBe("routing");

    const openHealthEventsButton = host?.querySelector(
      '[data-testid="open-health-events"]',
    );
    if (!(openHealthEventsButton instanceof HTMLButtonElement)) {
      throw new Error("missing open health events button");
    }

    act(() => {
      openHealthEventsButton.click();
    });

    expect(
      host?.querySelector('[data-testid="route-search"]')?.textContent,
    ).toBe("?upstreamAccountId=42&upstreamAccountTab=healthEvents");
    expect(
      host?.querySelector('[data-testid="route-account-tab"]')?.textContent,
    ).toBe("healthEvents");

    const openOverviewButton = host?.querySelector(
      '[data-testid="open-overview"]',
    );
    if (!(openOverviewButton instanceof HTMLButtonElement)) {
      throw new Error("missing open overview button");
    }

    act(() => {
      openOverviewButton.click();
    });

    expect(
      host?.querySelector('[data-testid="route-search"]')?.textContent,
    ).toBe("?upstreamAccountId=77");
    expect(
      host?.querySelector('[data-testid="route-account-tab"]')?.textContent,
    ).toBe("overview");

    const closeButton = host?.querySelector('[data-testid="close-account"]');
    if (!(closeButton instanceof HTMLButtonElement)) {
      throw new Error("missing close button");
    }

    act(() => {
      closeButton.click();
    });

    expect(
      host?.querySelector('[data-testid="route-search"]')?.textContent,
    ).toBe("");
    expect(
      host?.querySelector('[data-testid="route-account-id"]')?.textContent,
    ).toBe("null");
    expect(
      host?.querySelector('[data-testid="route-account-tab"]')?.textContent,
    ).toBe("overview");
  });
});
