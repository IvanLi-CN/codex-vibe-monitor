/** @vitest-environment jsdom */

import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useLatestDebouncedMutation } from "./useLatestDebouncedMutation";

let root: Root | null = null;
let host: HTMLDivElement | null = null;

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  root = null;
  host?.remove();
  host = null;
  vi.useRealTimers();
});

function renderProbe(
  mutate: (payload: string) => Promise<string>,
  onRevert?: (result: string | undefined) => void,
) {
  const controls: ReturnType<typeof useLatestDebouncedMutation<string, string>>[] = [];
  function Probe() {
    const mutation = useLatestDebouncedMutation({
      mutate,
      onRevert,
      resourceKey: "test-resource",
    });
    controls.push(mutation);
    return (
      <div>
        <output data-testid="status">{mutation.status}</output>
        <output data-testid="error">
          {mutation.error instanceof Error ? mutation.error.message : ""}
        </output>
      </div>
    );
  }
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(<Probe />);
  });
  return () => controls.at(-1)!;
}

describe("useLatestDebouncedMutation", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  it("sends one request with the final payload after the trailing delay", async () => {
    const mutate = vi.fn<(payload: string) => Promise<string>>().mockResolvedValue("saved");
    const getMutation = renderProbe(mutate);
    const mutation = getMutation();

    act(() => {
      mutation.schedule("first");
      mutation.schedule("second");
      mutation.schedule("final");
    });
    expect(mutate).not.toHaveBeenCalled();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(599);
    });
    expect(mutate).not.toHaveBeenCalled();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });
    expect(mutate).toHaveBeenCalledTimes(1);
    expect(mutate).toHaveBeenLastCalledWith("final");
  });

  it("serializes edits made while a request is in flight", async () => {
    let resolveFirst!: (value: string) => void;
    const first = new Promise<string>((resolve) => {
      resolveFirst = resolve;
    });
    const mutate = vi
      .fn<(payload: string) => Promise<string>>()
      .mockImplementationOnce(() => first)
      .mockResolvedValueOnce("second-saved");
    const getMutation = renderProbe(mutate);
    const mutation = getMutation();

    act(() => mutation.schedule("first"));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(600);
    });
    expect(mutate).toHaveBeenCalledTimes(1);

    act(() => {
      mutation.schedule("second");
      mutation.schedule("final");
    });
    expect(mutate).toHaveBeenCalledTimes(1);
    await act(async () => {
      resolveFirst("first-saved");
      await Promise.resolve();
    });
    expect(mutate).toHaveBeenCalledTimes(2);
    expect(mutate).toHaveBeenLastCalledWith("final");
  });

  it("flushes the latest queued payload immediately", async () => {
    const mutate = vi.fn<(payload: string) => Promise<string>>().mockResolvedValue("saved");
    const getMutation = renderProbe(mutate);
    const mutation = getMutation();

    act(() => mutation.schedule("latest"));
    await act(async () => {
      await mutation.flush();
    });
    expect(mutate).toHaveBeenCalledTimes(1);
    expect(mutate).toHaveBeenCalledWith("latest");
  });

  it("waits for the final in-flight follow-up when flushed", async () => {
    let resolveFirst!: (value: string) => void;
    const first = new Promise<string>((resolve) => {
      resolveFirst = resolve;
    });
    const mutate = vi
      .fn<(payload: string) => Promise<string>>()
      .mockImplementationOnce(() => first)
      .mockResolvedValueOnce("final-saved");
    const getMutation = renderProbe(mutate);
    const mutation = getMutation();

    act(() => mutation.schedule("first"));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(600);
    });
    act(() => mutation.schedule("final"));

    const flushPromise = mutation.flush();
    resolveFirst("first-saved");
    await act(async () => {
      await flushPromise;
    });

    expect(mutate).toHaveBeenNthCalledWith(2, "final");
    expect(mutate).toHaveBeenCalledTimes(2);
  });

  it("flushes the previous resource before processing a switched resource", async () => {
    let resolveFirst!: (value: string) => void;
    const first = new Promise<string>((resolve) => {
      resolveFirst = resolve;
    });
    const mutate = vi
      .fn<(payload: string) => Promise<string>>()
      .mockImplementationOnce(() => first)
      .mockResolvedValue("saved");
    const controls: ReturnType<typeof useLatestDebouncedMutation<string, string>>[] = [];
    let switchResource!: () => void;
    function ResourceProbe() {
      const [resource, setResource] = useState("first");
      switchResource = () => setResource("second");
      const mutation = useLatestDebouncedMutation({ mutate, resourceKey: resource });
      controls.push(mutation);
      return null;
    }
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
    act(() => root?.render(<ResourceProbe />));

    act(() => controls.at(-1)?.schedule("first-draft"));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(600);
    });
    act(() => controls.at(-1)?.schedule("first-follow-up"));
    act(switchResource);
    await act(async () => {
      resolveFirst("first-saved");
      await controls.at(-1)?.flush();
    });

    expect(mutate).toHaveBeenNthCalledWith(1, "first-draft");
    expect(mutate).toHaveBeenNthCalledWith(2, "first-follow-up");
  });

  it("retains a failed payload for retry and reverts to the confirmed result", async () => {
    const mutate = vi
      .fn<(payload: string) => Promise<string>>()
      .mockRejectedValueOnce(new Error("network down"))
      .mockResolvedValueOnce("retried");
    const onRevert = vi.fn<(result: string | undefined) => void>();
    const getMutation = renderProbe(mutate, onRevert);
    const mutation = getMutation();

    act(() => mutation.schedule("failed"));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(600);
    });
    expect(host?.querySelector('[data-testid="status"]')?.textContent).toBe("error");

    act(() => mutation.retry());
    await act(async () => {
      await vi.advanceTimersByTimeAsync(600);
    });
    expect(mutate).toHaveBeenLastCalledWith("failed");

    act(() => mutation.revert());
    expect(onRevert).toHaveBeenCalledWith("retried");
  });
});
