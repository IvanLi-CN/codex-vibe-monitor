/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { createMemoryRouter, RouterProvider } from "react-router-dom";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { ModelMappingNavigationBlocker } from "./UpstreamAccounts.page-local-shared";

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

async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

describe("ModelMappingNavigationBlocker", () => {
  it("blocks data-router navigation until a dirty mapping draft is discarded", async () => {
    let blocked: { proceed: () => void; reset: () => void } | null = null;
    const router = createMemoryRouter(
      [
        {
          path: "/routing",
          element: (
            <ModelMappingNavigationBlocker
              when
              onBlocked={(actions) => {
                blocked = actions;
              }}
            />
          ),
        },
        { path: "/other", element: <div>Other route</div> },
      ],
      { initialEntries: ["/routing"] },
    );
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
    act(() => {
      root?.render(<RouterProvider router={router} />);
    });

    void router.navigate("/other");
    await flush();
    expect(blocked).not.toBeNull();
    expect(router.state.location.pathname).toBe("/routing");

    act(() => {
      blocked?.reset();
    });
    await flush();
    expect(router.state.location.pathname).toBe("/routing");

    void router.navigate("/other");
    await flush();
    expect(blocked).not.toBeNull();
    act(() => {
      blocked?.proceed();
    });
    await flush();
    expect(router.state.location.pathname).toBe("/other");
  });
});
