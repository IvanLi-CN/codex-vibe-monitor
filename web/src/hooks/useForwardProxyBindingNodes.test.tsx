/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type { ForwardProxyBindingNode } from "../lib/api";
import { useForwardProxyBindingNodes } from "./useForwardProxyBindingNodes";

const apiMocks = vi.hoisted(() => ({
  fetchForwardProxyBindingNodes: vi.fn<
    (
      keys?: string[],
      options?: { includeCurrent?: boolean; groupName?: string },
    ) => Promise<ForwardProxyBindingNode[]>
  >(),
}));

vi.mock("../lib/api", async () => {
  const actual = await vi.importActual<typeof import("../lib/api")>("../lib/api");
  return {
    ...actual,
    fetchForwardProxyBindingNodes: apiMocks.fetchForwardProxyBindingNodes,
  };
});

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

function rerender(ui: React.ReactNode) {
  act(() => {
    root?.render(ui);
  });
}

async function flushAsync() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
  });
}

function Probe(props: {
  keys?: string[];
  enabled?: boolean;
  groupName?: string;
}) {
  const state = useForwardProxyBindingNodes(props.keys, {
    enabled: props.enabled,
    groupName: props.groupName,
  });
  return (
    <div>
      <div data-testid="count">{state.nodes.length}</div>
      <div data-testid="freshness">{state.catalogState.freshness}</div>
    </div>
  );
}

describe("useForwardProxyBindingNodes", () => {
  it("passes groupName to the binding nodes API when group-scoped stats are requested", async () => {
    apiMocks.fetchForwardProxyBindingNodes.mockResolvedValueOnce([]);

    render(<Probe keys={[" __direct__ "]} enabled groupName="  prod  " />);
    await flushAsync();

    expect(apiMocks.fetchForwardProxyBindingNodes).toHaveBeenCalledTimes(1);
    expect(apiMocks.fetchForwardProxyBindingNodes).toHaveBeenLastCalledWith(
      ["__direct__"],
      {
        includeCurrent: true,
        groupName: "prod",
      },
    );
  });

  it("treats groupName as part of the query identity so changing groups refetches", async () => {
    apiMocks.fetchForwardProxyBindingNodes
      .mockResolvedValueOnce([])
      .mockResolvedValueOnce([]);

    render(<Probe keys={["jp-edge-01"]} enabled groupName="prod" />);
    await flushAsync();

    rerender(<Probe keys={["jp-edge-01"]} enabled groupName="staging" />);
    await flushAsync();

    expect(apiMocks.fetchForwardProxyBindingNodes).toHaveBeenCalledTimes(2);
    expect(apiMocks.fetchForwardProxyBindingNodes).toHaveBeenNthCalledWith(
      1,
      ["jp-edge-01"],
      {
        includeCurrent: true,
        groupName: "prod",
      },
    );
    expect(apiMocks.fetchForwardProxyBindingNodes).toHaveBeenNthCalledWith(
      2,
      ["jp-edge-01"],
      {
        includeCurrent: true,
        groupName: "staging",
      },
    );
  });
});
