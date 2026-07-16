/** @vitest-environment jsdom */
import { act, useEffect } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useSubscriptionTopic } from "./useSubscriptionTopic";

const sseMocks = vi.hoisted(() => ({
  getCachedTopicState: vi.fn(),
  requestTopicRefresh: vi.fn(),
  subscribeToTopic: vi.fn(),
}));

vi.mock("../lib/sse", () => ({
  getCachedTopicState: sseMocks.getCachedTopicState,
  requestTopicRefresh: sseMocks.requestTopicRefresh,
  subscribeToTopic: sseMocks.subscribeToTopic,
}));

let host: HTMLDivElement | null = null;
let root: Root | null = null;

function HookHarness(props: {
  descriptor: { topic: string; params?: Record<string, string> } | null;
  enabled?: boolean;
  onRender: (snapshot: {
    data: { total: number } | null;
    isLoading: boolean;
    refresh: () => void;
  }) => void;
}) {
  const result = useSubscriptionTopic<{ total: number }>(props.descriptor, props.enabled ?? true);

  useEffect(() => {
    props.onRender({
      data: result.data,
      isLoading: result.isLoading,
      refresh: result.refresh,
    });
  }, [props, result.data, result.isLoading, result.refresh]);

  return null;
}

function renderHookHarness(props: {
  descriptor: { topic: string; params?: Record<string, string> } | null;
  enabled?: boolean;
  onRender: (snapshot: {
    data: { total: number } | null;
    isLoading: boolean;
    refresh: () => void;
  }) => void;
}) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(<HookHarness {...props} />);
  });
}

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
  document.body.innerHTML = "";
  sseMocks.getCachedTopicState.mockReset();
  sseMocks.requestTopicRefresh.mockReset();
  sseMocks.subscribeToTopic.mockReset();
});

describe("useSubscriptionTopic", () => {
  it("hydrates from live events and toggles loading around refresh", () => {
    const renders: Array<{
      data: { total: number } | null;
      isLoading: boolean;
      refresh: () => void;
    }> = [];
    let listener: ((event: { payload: { total: number } }) => void) | null = null;
    sseMocks.getCachedTopicState.mockReturnValue(null);
    sseMocks.subscribeToTopic.mockImplementation(
      (_descriptor: unknown, nextListener: (event: { payload: { total: number } }) => void) => {
        listener = nextListener;
        return () => {};
      },
    );

    renderHookHarness({
      descriptor: { topic: "stats.summary.current", params: { window: "current" } },
      onRender: (snapshot) => {
        renders.push(snapshot);
      },
    });

    expect(renders[0]?.data).toBeNull();
    expect(renders[0]?.isLoading).toBe(true);

    act(() => {
      listener?.({ payload: { total: 11 } });
    });

    expect(renders.at(-1)?.data).toEqual({ total: 11 });
    expect(renders.at(-1)?.isLoading).toBe(false);

    act(() => {
      renders.at(-1)?.refresh();
    });

    expect(sseMocks.requestTopicRefresh).toHaveBeenCalledWith({
      topic: "stats.summary.current",
      params: { window: "current" },
    });
    expect(renders.at(-1)?.isLoading).toBe(true);
  });

  it("stays disabled when descriptor is absent or the hook is disabled", () => {
    const renders: Array<{ data: { total: number } | null; isLoading: boolean }> = [];

    renderHookHarness({
      descriptor: null,
      onRender: (snapshot) => {
        renders.push(snapshot);
      },
    });

    expect(sseMocks.subscribeToTopic).not.toHaveBeenCalled();
    expect(renders.at(-1)).toMatchObject({ data: null, isLoading: false });

    renders.length = 0;
    sseMocks.getCachedTopicState.mockReturnValue({ payload: { total: 99 } });

    act(() => {
      root?.render(
        <HookHarness
          descriptor={{ topic: "stats.summary.current" }}
          enabled={false}
          onRender={(snapshot) => {
            renders.push(snapshot);
          }}
        />,
      );
    });

    expect(sseMocks.subscribeToTopic).not.toHaveBeenCalled();
    expect(renders.at(-1)).toMatchObject({ data: null, isLoading: false });
  });
});
