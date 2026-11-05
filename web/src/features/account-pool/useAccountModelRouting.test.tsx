/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ModelRoutingState, UpstreamAccountDetail } from "../../lib/api";
import { fetchUpstreamAccountModelRouting } from "../../lib/api/core-upstream";
import { useAccountModelRouting } from "./useAccountModelRouting";

vi.mock("../../lib/api/core-upstream", () => ({ fetchUpstreamAccountModelRouting: vi.fn() }));
const fetchStates = vi.mocked(fetchUpstreamAccountModelRouting);
const host = document.createElement("div");
let root = createRoot(host);
function Harness({ id }: { id: number }) {
  const result = useAccountModelRouting(details[id]);
  return <output>{JSON.stringify(result)}</output>;
}
const details = {
  1: { id: 1, kind: "api_key_codex" },
  2: { id: 2, kind: "api_key_codex" },
  3: { id: 3, kind: "oauth_codex" },
} as Record<number, UpstreamAccountDetail>;
afterEach(() => {
  act(() => root.unmount());
  root = createRoot(host);
  vi.resetAllMocks();
});
describe("complete model routing loading", () => {
  it("loads the full model union and discards stale results across accounts", async () => {
    let finishFirst!: (value: ModelRoutingState[]) => void;
    fetchStates.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finishFirst = resolve;
        }),
    );
    fetchStates.mockResolvedValueOnce([
      {
        model: "catalog-only",
        state: "unknown",
        priority: "normal",
        failureCount: 0,
        lastSeenAt: "unknown",
      },
    ]);
    await act(async () => root.render(<Harness id={1} />));
    expect(host.textContent).toContain('"loading":true');
    await act(async () => root.render(<Harness id={2} />));
    await act(async () =>
      finishFirst([
        {
          model: "stale-model",
          state: "available",
          priority: "normal",
          failureCount: 0,
          lastSeenAt: "unknown",
        },
      ]),
    );
    expect(host.textContent).toContain("catalog-only");
    expect(host.textContent).not.toContain("stale-model");
    expect(fetchStates.mock.calls[0][1]?.aborted).toBe(true);
  });
  it("loads OAuth model health from the same complete account model API", async () => {
    fetchStates.mockResolvedValueOnce([
      {
        model: "oauth-model",
        state: "cooling_down",
        priority: "excluded",
        failureCount: 5,
        lastSeenAt: "2026-11-05T00:00:00Z",
      },
    ]);
    await act(async () => root.render(<Harness id={3} />));
    expect(fetchStates).toHaveBeenCalledWith(3, expect.any(AbortSignal));
    expect(host.textContent).toContain("oauth-model");
    expect(host.textContent).toContain('"loading":false');
  });
  it("surfaces request errors without claiming an empty successful model set", async () => {
    fetchStates.mockRejectedValue(new Error("routing unavailable"));
    await act(async () => root.render(<Harness id={1} />));
    expect(host.textContent).toContain('"loading":false');
    expect(host.textContent).toContain("routing unavailable");
  });
});
