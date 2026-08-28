import { describe, expect, it } from "vitest";
import {
  buildInvocationCompactTiming,
  reconcileInvocationCompactTiming,
} from "./invocationCompactTiming";

const baseRecord = {
  tReqReadMs: 10,
  tReqParseMs: 7,
  tUpstreamConnectMs: 90,
  tUpstreamTtfbMs: 70,
  firstTokenMs: null,
  tUpstreamStreamMs: null,
};

describe("buildInvocationCompactTiming", () => {
  it("shows only a live request timer before the first upstream byte", () => {
    expect(
      buildInvocationCompactTiming({
        record: { ...baseRecord, tUpstreamTtfbMs: null },
        occurredAtEpoch: 1_000,
        isInFlight: true,
        nowMs: 2_234,
      }),
    ).toMatchObject({
      state: "requesting",
      requestMs: 1_234,
      requestProvisional: true,
      ttftMs: null,
      responseMs: null,
    });
  });

  it("shows the authoritative request time and provisional TTFT after the first byte", () => {
    expect(
      buildInvocationCompactTiming({
        record: baseRecord,
        occurredAtEpoch: 1_000,
        isInFlight: true,
        nowMs: 2_234,
      }),
    ).toMatchObject({ requestMs: 177, ttftMs: 1_234, state: "awaitingToken" });
  });

  it("treats zero milliseconds as a valid measured TTFT", () => {
    expect(
      buildInvocationCompactTiming({
        record: { ...baseRecord, firstTokenMs: 0 },
        occurredAtEpoch: 1_000,
        isInFlight: true,
        nowMs: 2_234,
      }),
    ).toMatchObject({ state: "responding", ttftMs: 0, responseMs: 1_057 });
  });

  it("keeps tokenless terminal calls strict while retaining a valid response duration", () => {
    expect(
      buildInvocationCompactTiming({
        record: { ...baseRecord, tUpstreamStreamMs: 1_200 },
        occurredAtEpoch: 1_000,
        isInFlight: false,
        nowMs: 2_234,
      }),
    ).toMatchObject({ state: "terminal", ttftMs: null, responseMs: 1_200 });
  });

  it("does not let TTFB or total time manufacture strict TTFT", () => {
    expect(
      buildInvocationCompactTiming({
        record: {
          ...baseRecord,
          tUpstreamTtfbMs: 0,
          tUpstreamStreamMs: 2_000,
        },
        occurredAtEpoch: 1_000,
        isInFlight: false,
        nowMs: 2_234,
      }),
    ).toMatchObject({ state: "terminal", requestMs: null, ttftMs: null, responseMs: null });
  });
});

describe("reconcileInvocationCompactTiming", () => {
  it("freezes a local value while the server anchor catches up", () => {
    const previous = {
      requestMs: 1_500,
      requestProvisional: true,
      ttftMs: null,
      ttftProvisional: false,
      responseMs: null,
      responseProvisional: false,
    };
    const serverUpdate = {
      state: "awaitingToken" as const,
      requestMs: 1_000,
      requestProvisional: false,
      ttftMs: 1_700,
      ttftProvisional: true,
      responseMs: null,
      responseProvisional: false,
    };
    expect(reconcileInvocationCompactTiming(serverUpdate, previous)).toMatchObject({
      requestMs: 1_500,
      requestProvisional: true,
    });

    expect(
      reconcileInvocationCompactTiming({ ...serverUpdate, requestMs: 1_600 }, previous),
    ).toMatchObject({ requestMs: 1_600, requestProvisional: false });
  });

  it("jumps directly to terminal values", () => {
    const terminal = {
      state: "terminal" as const,
      requestMs: 177,
      requestProvisional: false,
      ttftMs: null,
      ttftProvisional: false,
      responseMs: 1_200,
      responseProvisional: false,
    };
    expect(
      reconcileInvocationCompactTiming(terminal, {
        requestMs: 3_000,
        requestProvisional: true,
        ttftMs: 3_000,
        ttftProvisional: true,
        responseMs: 2_000,
        responseProvisional: true,
      }),
    ).toEqual({
      requestMs: 177,
      requestProvisional: false,
      ttftMs: null,
      ttftProvisional: false,
      responseMs: 1_200,
      responseProvisional: false,
    });
  });
});
