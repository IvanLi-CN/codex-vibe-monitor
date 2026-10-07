import { afterEach, describe, expect, it, vi } from "vitest";
import {
  acceptsRoutingStateVersion,
  compareRoutingStateVersion,
  fetchManagedTask,
  fetchManagedTaskTimeline,
  fetchSystemStatus,
  fetchSystemStorage,
  normalizePoolRoutingSelectionAudit,
  releaseInvocationTimelineSnapshot,
} from "./core-foundation";

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("fetchManagedTaskTimeline response contract", () => {
  const validPage = {
    observedAt: "2026-10-07T00:00:00Z",
    windowStart: "2026-10-06T12:00:00Z",
    windowEnd: "2026-10-07T00:00:00Z",
    watermark: 7,
    segments: [
      {
        segmentId: "run-1",
        kind: "execution",
        taskKey: "retention_archive",
        title: "Retention archive",
        startedAt: "2026-10-06T23:00:00Z",
        lastObservedAt: "2026-10-06T23:01:00Z",
        finishedAt: "2026-10-06T23:01:00Z",
        durationMs: 60_000,
        status: "success",
        sessionId: "session-1",
        revision: 7,
      },
    ],
    coverage: [],
    nextCursor: "cursor-1",
    resetRequired: false,
  };

  it("returns a complete normalized page without changing its pagination cursor", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response(JSON.stringify(validPage), { status: 200 })),
    );

    const page = await fetchManagedTaskTimeline({ limit: 500 });

    expect(page).toMatchObject({
      watermark: 7,
      nextCursor: "cursor-1",
      resetRequired: false,
      segments: [expect.objectContaining({ segmentId: "run-1", revision: 7 })],
    });
  });

  it.each(["null", "absent"])("accepts %s optional timeline timestamps", async (presence) => {
    const segmentWithOptionalTimestamps: Record<string, unknown> = {
      ...validPage.segments[0],
    };
    const coverageWithOptionalTimestamp: Record<string, unknown> = {
      sessionId: "session-1",
      startedAt: "2026-10-06T23:00:00Z",
      lastSeenAt: "2026-10-06T23:01:00Z",
      droppedEvents: 0,
    };
    if (presence === "null") {
      segmentWithOptionalTimestamps.finishedAt = null;
      segmentWithOptionalTimestamps.retryAt = null;
      Object.assign(coverageWithOptionalTimestamp, { endedAt: null });
    } else {
      delete segmentWithOptionalTimestamps.finishedAt;
      delete segmentWithOptionalTimestamps.retryAt;
    }

    vi.stubGlobal(
      "fetch",
      vi.fn(
        async () =>
          new Response(
            JSON.stringify({
              ...validPage,
              segments: [segmentWithOptionalTimestamps],
              coverage: [coverageWithOptionalTimestamp],
            }),
            { status: 200 },
          ),
      ),
    );

    const page = await fetchManagedTaskTimeline({ limit: 500 });
    expect(page.segments[0]).toMatchObject({ finishedAt: null, retryAt: null });
    expect(page.coverage[0]).toMatchObject({ endedAt: null });
  });

  it.each([
    ["missing segments", { segments: undefined }],
    ["missing cursor", { nextCursor: undefined }],
    ["invalid cursor", { nextCursor: 42 }],
    ["invalid watermark", { watermark: "7" }],
    ["invalid segment", { segments: [{ ...validPage.segments[0], revision: undefined }] }],
    [
      "invalid segment finish timestamp",
      {
        segments: [
          validPage.segments[0],
          { ...validPage.segments[0], segmentId: "run-2", finishedAt: "bad" },
        ],
      },
    ],
    [
      "invalid segment retry timestamp",
      {
        segments: [
          validPage.segments[0],
          { ...validPage.segments[0], segmentId: "run-2", retryAt: "bad" },
        ],
      },
    ],
    [
      "invalid coverage end timestamp",
      {
        coverage: [
          {
            sessionId: "session-1",
            startedAt: "2026-10-06T23:00:00Z",
            lastSeenAt: "2026-10-06T23:01:00Z",
            endedAt: null,
            droppedEvents: 0,
          },
          {
            sessionId: "session-2",
            startedAt: "2026-10-06T23:00:00Z",
            lastSeenAt: "2026-10-06T23:01:00Z",
            endedAt: "bad",
            droppedEvents: 0,
          },
        ],
      },
    ],
  ])("rejects a 200 response with %s instead of accepting a partial page", async (_label, changes) => {
    vi.stubGlobal(
      "fetch",
      vi.fn(
        async () => new Response(JSON.stringify({ ...validPage, ...changes }), { status: 200 }),
      ),
    );

    await expect(fetchManagedTaskTimeline({ limit: 500 })).rejects.toThrow(
      "Invalid managed task timeline response",
    );
  });
});

describe("retention throughput optional contract", () => {
  it("keeps old runs unknown and normalizes valid zero without accepting malformed batches", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(
        async () =>
          new Response(
            JSON.stringify({
              task: {
                taskKey: "retention_archive",
                title: "Retention",
                description: "Archive eligible records",
                triggerMode: "interval",
                enabled: true,
                isManual: false,
              },
              recentRuns: [
                { id: 1, startedAt: "2026-10-04T00:00:00Z", status: "success" },
                {
                  id: 2,
                  startedAt: "2026-10-04T01:00:00Z",
                  status: "success",
                  details: {
                    timeoutCount: 0,
                    archiveBatches: [
                      {
                        dataset: "codex_invocations",
                        monthKey: "2026-09",
                        committedRows: 0,
                        timeoutCount: 0,
                        arrivalRowsPerSecond: 0,
                        committedRowsPerSecond: "bad",
                        serviceRateMultiple: -1,
                      },
                      { dataset: 12, monthKey: "2026-09" },
                      {
                        dataset: "pool_upstream_request_attempts",
                        monthKey: "2026-09",
                        timeoutCount: 1,
                      },
                      { dataset: "codex_invocations", monthKey: "2026-08" },
                      {
                        dataset: "codex_invocations",
                        monthKey: "2026-07",
                        timeoutCount: -1,
                      },
                    ],
                  },
                },
              ],
            }),
            { status: 200 },
          ),
      ),
    );
    const detail = await fetchManagedTask("retention_archive");
    expect(detail.recentRuns[0].details).toBeUndefined();
    expect(detail.recentRuns[1].details?.archiveBatches).toHaveLength(4);
    expect(detail.recentRuns[1].details?.archiveBatches?.[0]).toMatchObject({
      committedRows: 0,
      timeoutCount: 0,
      arrivalRowsPerSecond: 0,
      committedRowsPerSecond: null,
      serviceRateMultiple: null,
    });
    expect(detail.recentRuns[1].details?.archiveBatches?.[1]).toMatchObject({
      dataset: "pool_upstream_request_attempts",
      timeoutCount: 1,
    });
    expect(detail.recentRuns[1].details?.archiveBatches?.[2].timeoutCount).toBeNull();
    expect(detail.recentRuns[1].details?.archiveBatches?.[3].timeoutCount).toBeNull();
  });
});

describe("releaseInvocationTimelineSnapshot", () => {
  it("releases the opaque snapshot token with DELETE and accepts an empty 204 response", async () => {
    const fetchMock = vi.fn(async () => new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", fetchMock);

    await expect(releaseInvocationTimelineSnapshot("snapshot/with space")).resolves.toBeUndefined();

    expect(fetchMock).toHaveBeenCalledWith(
      "/api/stats/invocation-timeline/snapshot%2Fwith%20space",
      expect.objectContaining({ method: "DELETE" }),
    );
  });
});

describe("fetchSystemStatus retention recovery compatibility", () => {
  it("keeps raw bytes unknown while the inventory is not ready", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(
        async () =>
          new Response(
            JSON.stringify({
              rawBodies: { count: 4, bytes: 0 },
              requestRawBodies: { count: 2, bytes: 0 },
              responseRawBodies: { count: 2, bytes: 0 },
              rawMetricsHealth: {
                state: "preparing",
                inventoryCursor: 12,
                physicalCoverage: "unknown",
              },
            }),
            { status: 200 },
          ),
      ),
    );

    const status = await fetchSystemStatus();

    expect(status.rawMetricsHealth).toMatchObject({
      state: "preparing",
      inventoryCursor: 12,
      physicalCoverage: "unknown",
    });
    expect(status.rawBodies.bytes).toBeNull();
    expect(status.requestRawBodies.bytes).toBeNull();
    expect(status.responseRawBodies.bytes).toBeNull();
  });

  it("preserves ready tracked bytes without assuming complete physical coverage", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(
        async () =>
          new Response(
            JSON.stringify({
              rawBodies: { count: 4, bytes: 17 },
              requestRawBodies: { count: 2, bytes: 11 },
              responseRawBodies: { count: 2, bytes: 9 },
              rawMetricsHealth: { state: "ready", inventoryCursor: 12 },
            }),
            { status: 200 },
          ),
      ),
    );

    const status = await fetchSystemStatus();

    expect(status.rawBodies.bytes).toBe(17);
    expect(status.requestRawBodies.bytes).toBe(11);
    expect(status.responseRawBodies.bytes).toBe(9);
    expect(status.rawMetricsHealth.physicalCoverage).toBe("unknown");
  });

  it("normalizes a missing recovery diagnostic to unknown for older backends", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(
        async () => new Response(JSON.stringify({ runtimePressureHealth: {} }), { status: 200 }),
      ),
    );

    const status = await fetchSystemStatus();

    expect(status.runtimePressureHealth?.retentionRecovery).toEqual({
      state: "unknown",
      stage: undefined,
      preparedCount: undefined,
      quarantinedCount: undefined,
      expiredBacklogCount: undefined,
      oldestBacklogAgeSecs: undefined,
      lastProgressAt: undefined,
      nextRetryAt: undefined,
      failureStage: undefined,
      failureFingerprint: undefined,
      deferReason: undefined,
      consecutiveFailureCount: undefined,
    });
  });

  it("normalizes a missing raw orphan sweep diagnostic to unknown", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(
        async () => new Response(JSON.stringify({ runtimePressureHealth: {} }), { status: 200 }),
      ),
    );

    const status = await fetchSystemStatus();

    expect(status.runtimePressureHealth?.rawOrphanSweep).toEqual({
      state: "unknown",
      inspectedEntries: undefined,
      referencedSkipped: undefined,
      quarantined: undefined,
      removed: undefined,
      removedBytes: undefined,
      lastProgressAt: undefined,
      nextRetryAt: undefined,
      deferReason: undefined,
      failureFingerprint: undefined,
      admissionStage: undefined,
      admissionCause: undefined,
      lastSettledPass: undefined,
      lastNonzeroRemoval: undefined,
    });
  });

  it("keeps raw orphan sweep counters unknown when only a partial diagnostic arrives", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(
        async () =>
          new Response(
            JSON.stringify({
              runtimePressureHealth: {
                rawOrphanSweep: {
                  state: "deferred",
                  inspectedEntries: 128,
                  deferReason: "sqlite_pressure",
                  failureFingerprint: "7d38a1c0b4c8e2f1",
                },
              },
            }),
            { status: 200 },
          ),
      ),
    );

    const sweep = (await fetchSystemStatus()).runtimePressureHealth?.rawOrphanSweep;

    expect(sweep?.state).toBe("deferred");
    expect(sweep?.inspectedEntries).toBe(128);
    expect(sweep?.referencedSkipped).toBeUndefined();
    expect(sweep?.removed).toBeUndefined();
    expect(sweep?.failureFingerprint).toBe("7d38a1c0b4c8e2f1");
  });

  it("rejects untrusted raw orphan sweep fingerprints", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(
        async () =>
          new Response(
            JSON.stringify({
              runtimePressureHealth: {
                rawOrphanSweep: { state: "degraded", failureFingerprint: "/private/raw/path" },
              },
            }),
            { status: 200 },
          ),
      ),
    );

    const sweep = (await fetchSystemStatus()).runtimePressureHealth?.rawOrphanSweep;

    expect(sweep?.failureFingerprint).toBeUndefined();
  });

  it("normalizes durable settled-pass evidence and fixed admission causes", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(
        async () =>
          new Response(
            JSON.stringify({
              runtimePressureHealth: {
                rawOrphanSweep: {
                  state: "deferred",
                  removedBytes: 4096,
                  admissionStage: "background_slot",
                  admissionCause: "background_busy",
                  lastSettledPass: {
                    settledAt: "2026-09-26T02:00:00Z",
                    complete: false,
                    inspectedEntries: 128,
                    removed: 1,
                    removedBytes: 4096,
                  },
                  lastNonzeroRemoval: {
                    removedAt: "2026-09-26T01:59:00Z",
                    removed: 1,
                    removedBytes: 4096,
                  },
                },
              },
            }),
            { status: 200 },
          ),
      ),
    );

    const sweep = (await fetchSystemStatus()).runtimePressureHealth?.rawOrphanSweep;

    expect(sweep?.removedBytes).toBe(4096);
    expect(sweep?.admissionStage).toBe("background_slot");
    expect(sweep?.admissionCause).toBe("background_busy");
    expect(sweep?.lastSettledPass).toMatchObject({ complete: false, removedBytes: 4096 });
    expect(sweep?.lastNonzeroRemoval).toMatchObject({ removed: 1, removedBytes: 4096 });
  });

  it("rejects unknown raw orphan admission values", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(
        async () =>
          new Response(
            JSON.stringify({
              runtimePressureHealth: {
                rawOrphanSweep: {
                  state: "deferred",
                  admissionStage: "/private/raw/path",
                  admissionCause: "sql=secret",
                },
              },
            }),
            { status: 200 },
          ),
      ),
    );

    const sweep = (await fetchSystemStatus()).runtimePressureHealth?.rawOrphanSweep;

    expect(sweep?.admissionStage).toBeUndefined();
    expect(sweep?.admissionCause).toBeUndefined();
  });

  it("keeps omitted recovery counters unknown when a partial diagnostic arrives", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(
        async () =>
          new Response(
            JSON.stringify({
              runtimePressureHealth: {
                retentionRecovery: { state: "recovering", preparedCount: 5 },
              },
            }),
            { status: 200 },
          ),
      ),
    );

    const status = await fetchSystemStatus();
    const recovery = status.runtimePressureHealth?.retentionRecovery;

    expect(recovery?.state).toBe("recovering");
    expect(recovery?.preparedCount).toBe(5);
    expect(recovery?.quarantinedCount).toBeUndefined();
    expect(recovery?.expiredBacklogCount).toBeUndefined();
  });

  it("normalizes explicitly unavailable recovery counters to unknown", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(
        async () =>
          new Response(
            JSON.stringify({
              runtimePressureHealth: {
                retentionRecovery: {
                  state: "unknown",
                  preparedCount: null,
                  quarantinedCount: null,
                  expiredBacklogCount: null,
                },
              },
            }),
            { status: 200 },
          ),
      ),
    );

    const recovery = (await fetchSystemStatus()).runtimePressureHealth?.retentionRecovery;

    expect(recovery?.preparedCount).toBeUndefined();
    expect(recovery?.quarantinedCount).toBeUndefined();
    expect(recovery?.expiredBacklogCount).toBeUndefined();
  });

  it("normalizes raw reference timing as an optional non-negative duration", async () => {
    const fetchStatus = async (rawReferenceCheckMs: unknown) => {
      vi.stubGlobal(
        "fetch",
        vi.fn(
          async () =>
            new Response(
              JSON.stringify({
                runtimePressureHealth: {
                  retentionWriteHealth: { state: "healthy", rawReferenceCheckMs },
                },
              }),
              { status: 200 },
            ),
        ),
      );
      return fetchSystemStatus();
    };

    expect(
      (await fetchStatus(-1)).runtimePressureHealth?.retentionWriteHealth?.rawReferenceCheckMs,
    ).toBeUndefined();
    expect(
      (await fetchStatus(null)).runtimePressureHealth?.retentionWriteHealth?.rawReferenceCheckMs,
    ).toBeUndefined();
    expect(
      (await fetchStatus(4.5)).runtimePressureHealth?.retentionWriteHealth?.rawReferenceCheckMs,
    ).toBe(4.5);
  });
});

describe("fetchSystemStorage", () => {
  it("normalizes independent storage state and permits a real zero-byte result", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(
        async () =>
          new Response(
            JSON.stringify({
              totalBytes: 0,
              sampledAt: "2026-06-22T08:00:00Z",
              state: "ready",
              scanInProgress: false,
              stale: false,
              reason: null,
            }),
            { status: 200 },
          ),
      ),
    );

    await expect(fetchSystemStorage()).resolves.toEqual({
      totalBytes: 0,
      sampledAt: "2026-06-22T08:00:00Z",
      state: "ready",
      scanInProgress: false,
      stale: false,
      reason: null,
    });
  });

  it("rejects a missing or malformed endpoint response instead of inventing a total", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response("{}", { status: 200 })),
    );
    await expect(fetchSystemStorage()).rejects.toThrow("invalid system storage response");

    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response("not found", { status: 404 })),
    );
    await expect(fetchSystemStorage()).rejects.toThrow("Request failed: 404");
  });
});

describe("normalizePoolRoutingSelectionAudit", () => {
  it("preserves the optional recovery trigger", () => {
    const audit = normalizePoolRoutingSelectionAudit({
      selectedAccountId: 2918,
      selectedAccountName: "Ciii2",
      eligibleCandidateCount: 2,
      winnerReasonCode: "requestDrivenRecoveryAdmission",
      handoffAdmission: {
        decision: "admitted",
        phase: "verifying",
        verificationSuccessCount: 1,
        generation: 8,
        trigger: "modelRouteRecovery",
      },
      excludedCandidates: [],
    });

    expect(audit?.handoffAdmission?.trigger).toBe("modelRouteRecovery");
  });

  it("keeps historical admissions valid when trigger is absent", () => {
    const audit = normalizePoolRoutingSelectionAudit({
      selectedAccountId: 11,
      selectedAccountName: "Aster",
      eligibleCandidateCount: 1,
      winnerReasonCode: "onlyEligibleCandidate",
      handoffAdmission: {
        decision: "admitted",
        phase: "verifying",
        verificationSuccessCount: 0,
      },
      excludedCandidates: [],
    });

    expect(audit?.handoffAdmission).toEqual({
      decision: "admitted",
      phase: "verifying",
      verificationSuccessCount: 0,
    });
  });
});

describe("RoutingStateVersion", () => {
  const epoch = "2026-09-05T12:00:00.000000000Z";
  const older = { epoch, generation: "9" };
  const newer = { epoch, generation: "10" };
  const restarted = { epoch: "2026-09-05T12:00:01.000000000Z", generation: "1" };

  it("orders generations numerically within one process epoch", () => {
    expect(compareRoutingStateVersion(older, newer)).toBe(-1);
    expect(compareRoutingStateVersion(newer, older)).toBe(1);
    expect(acceptsRoutingStateVersion(newer, older, "live")).toBe(false);
    expect(acceptsRoutingStateVersion(older, newer, "live")).toBe(true);
  });

  it("only allows a cross-epoch snapshot or patch to replace the fence", () => {
    expect(acceptsRoutingStateVersion(newer, restarted, "live")).toBe(false);
    expect(acceptsRoutingStateVersion(newer, restarted, "snapshot")).toBe(true);
    expect(acceptsRoutingStateVersion(newer, restarted, "patch")).toBe(true);
  });

  it("allows a nullable patch without dropping an existing confirmation fence", () => {
    expect(acceptsRoutingStateVersion(newer, null, "patch")).toBe(true);
    expect(acceptsRoutingStateVersion(newer, null, "live")).toBe(false);
  });
});
