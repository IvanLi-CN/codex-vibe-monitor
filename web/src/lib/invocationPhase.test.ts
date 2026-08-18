import { describe, expect, it } from "vitest";
import { resolveInvocationLivePhase, sumInvocationPhaseCounts } from "./invocationPhase";

describe("resolveInvocationLivePhase", () => {
  it("uses the backend live phase for in-flight invocations after first token", () => {
    expect(
      resolveInvocationLivePhase({
        status: "running",
        failureClass: "none",
        livePhase: "responding",
        firstTokenMs: 42,
      }),
    ).toBe("responding");
  });

  it("uses measured first-token time before declaring a running invocation responding", () => {
    expect(
      resolveInvocationLivePhase({
        status: "running",
        failureClass: "none",
        tUpstreamTtfbMs: 42,
      }),
    ).toBe("queued");
    expect(
      resolveInvocationLivePhase({
        status: "running",
        failureClass: "none",
        firstTokenMs: 42,
      }),
    ).toBe("responding");
  });

  it("treats a measured zero-millisecond first token as response progress", () => {
    expect(
      resolveInvocationLivePhase({
        status: "running",
        failureClass: "none",
        firstTokenMs: 0,
      }),
    ).toBe("responding");
    expect(
      resolveInvocationLivePhase({
        status: "running",
        failureClass: "none",
        tReqReadMs: 2,
        tUpstreamTtfbMs: 0,
      }),
    ).toBe("requesting");
  });

  it("downgrades an inconsistent explicit responding phase until first-token timing arrives", () => {
    expect(
      resolveInvocationLivePhase({
        status: "running",
        failureClass: "none",
        livePhase: "responding",
        tUpstreamConnectMs: 12,
        firstTokenMs: null,
      }),
    ).toBe("requesting");
  });

  it("promotes a measured first token over stale pre-response phase metadata", () => {
    expect(
      resolveInvocationLivePhase({
        status: "running",
        failureClass: "none",
        livePhase: "requesting",
        firstTokenMs: 42,
      }),
    ).toBe("responding");
  });

  it("keeps pending records queued when stale phase metadata is present", () => {
    expect(
      resolveInvocationLivePhase({
        status: "pending",
        failureClass: "none",
        livePhase: "responding",
        upstreamAccountId: "account-1",
        firstTokenMs: null,
      }),
    ).toBe("queued");
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
