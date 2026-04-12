/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ForwardProxyCatalogState } from "../../hooks/useUpstreamAccounts";
import { useGroupNoteCatalogAutoRefresh } from "./useGroupNoteCatalogAutoRefresh";

let host: HTMLDivElement | null = null;
let root: Root | null = null;

function render(ui: React.ReactNode) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(ui);
  });
}

function rerender(ui: React.ReactNode) {
  act(() => {
    root?.render(ui);
  });
}

function createCatalogState(
  overrides: Partial<ForwardProxyCatalogState>,
): ForwardProxyCatalogState {
  return {
    kind: "ready-empty",
    freshness: "fresh",
    isPending: false,
    hasNodes: false,
    ...overrides,
  };
}

function Probe(props: {
  open: boolean;
  refresh: (options?: { silent?: boolean }) => Promise<unknown>;
  catalogState: ForwardProxyCatalogState;
}) {
  useGroupNoteCatalogAutoRefresh({
    open: props.open,
    refresh: props.refresh,
    catalogState: props.catalogState,
  });
  return null;
}

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
});

describe("useGroupNoteCatalogAutoRefresh", () => {
  it("does not launch a second refresh while an empty catalog refresh is already loading", () => {
    const refresh = vi.fn<
      (options?: { silent?: boolean }) => Promise<unknown>
    >(() => Promise.resolve());

    render(
      <Probe
        open
        refresh={refresh}
        catalogState={createCatalogState({
          kind: "loading",
          freshness: "stale",
          isPending: true,
        })}
      />,
    );

    expect(refresh).not.toHaveBeenCalled();
  });

  it("avoids retry loops when the catalog falls back to missing after a failed refresh", () => {
    const refresh = vi.fn<
      (options?: { silent?: boolean }) => Promise<unknown>
    >(() => Promise.resolve());

    render(
      <Probe
        open
        refresh={refresh}
        catalogState={createCatalogState({
          kind: "missing",
          freshness: "missing",
        })}
      />,
    );

    expect(refresh).toHaveBeenCalledTimes(1);
    expect(refresh).toHaveBeenLastCalledWith({ silent: true });

    rerender(
      <Probe
        open
        refresh={refresh}
        catalogState={createCatalogState({
          kind: "loading",
          freshness: "missing",
          isPending: true,
        })}
      />,
    );
    rerender(
      <Probe
        open
        refresh={refresh}
        catalogState={createCatalogState({
          kind: "missing",
          freshness: "missing",
        })}
      />,
    );

    expect(refresh).toHaveBeenCalledTimes(1);
  });

  it("retries again after the dialog recovers or reopens", () => {
    const refresh = vi.fn<
      (options?: { silent?: boolean }) => Promise<unknown>
    >(() => Promise.resolve());

    render(
      <Probe
        open
        refresh={refresh}
        catalogState={createCatalogState({
          kind: "missing",
          freshness: "missing",
        })}
      />,
    );
    expect(refresh).toHaveBeenCalledTimes(1);

    rerender(
      <Probe
        open
        refresh={refresh}
        catalogState={createCatalogState({
          kind: "ready-with-data",
          freshness: "fresh",
          hasNodes: true,
        })}
      />,
    );
    rerender(
      <Probe
        open
        refresh={refresh}
        catalogState={createCatalogState({
          kind: "ready-with-data",
          freshness: "stale",
          hasNodes: true,
        })}
      />,
    );
    expect(refresh).toHaveBeenCalledTimes(2);

    rerender(
      <Probe
        open={false}
        refresh={refresh}
        catalogState={createCatalogState({
          kind: "ready-with-data",
          freshness: "fresh",
          hasNodes: true,
        })}
      />,
    );
    rerender(
      <Probe
        open
        refresh={refresh}
        catalogState={createCatalogState({
          kind: "missing",
          freshness: "missing",
        })}
      />,
    );
    expect(refresh).toHaveBeenCalledTimes(3);
  });
});
