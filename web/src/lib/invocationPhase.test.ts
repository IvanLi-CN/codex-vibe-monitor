import { describe, expect, it } from "vitest";
import { resolveInvocationLivePhase, sumInvocationPhaseCounts } from "./invocationPhase";

describe("resolveInvocationLivePhase", () => {
  it("uses the backend live phase for in-flight invocations", () => {
    expect(
      resolveInvocationLivePhase({
        status: "running",
        failureClass: "none",
        livePhase: "responding",
      }),
    ).toBe("responding");
  });

  it("falls back to timings for non-pool running invocations", () => {
    expect(
      resolveInvocationLivePhase({
        status: "running",
        failureClass: "none",
        tUpstreamTtfbMs: 42,
      }),
    ).toBe("responding");
    expect(
      resolveInvocationLivePhase({
        status: "running",
        failureClass: "none",
        tReqParseMs: 3,
      }),
    ).toBe("requesting");
  });

  it("does not treat zero placeholder timings as response progress", () => {
    expect(
      resolveInvocationLivePhase({
        status: "running",
        failureClass: "none",
        tUpstreamTtfbMs: 0,
        tUpstreamStreamMs: 0,
      }),
    ).toBe("queued");
    expect(
      resolveInvocationLivePhase({
        status: "running",
        failureClass: "none",
        tReqReadMs: 2,
        tUpstreamTtfbMs: 0,
      }),
    ).toBe("requesting");
  });

  it("keeps terminal or resolved-failure rows out of the live phase model", () => {
    expect(
      resolveInvocationLivePhase({
        status: "success",
        failureClass: "none",
        livePhase: "responding",
      }),
    ).toBeNull();
    expect(
      resolveInvocationLivePhase({
        status: "running",
        failureClass: "service_failure",
        livePhase: "responding",
      }),
    ).toBeNull();
  });
});

describe("sumInvocationPhaseCounts", () => {
  it("sums backend account-level phase counts without reading visible rows", () => {
    expect(sumInvocationPhaseCounts({ queued: 2, requesting: 3, responding: 4 })).toBe(9);
  });
});
