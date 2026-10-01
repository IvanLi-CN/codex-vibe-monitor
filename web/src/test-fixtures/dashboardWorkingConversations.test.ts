import { describe, expect, it } from "vitest";
import {
  buildDashboardActivityResponse,
  buildSummary,
  buildTimeseries,
  buildWorkingConversationsResponse,
  createConversation,
  createPreview,
} from "./dashboardWorkingConversations";

describe("Dashboard working conversation fixtures", () => {
  it("keeps running, pending, and terminal invocation shapes distinct", () => {
    const response = buildWorkingConversationsResponse();
    const running = response.conversations[0];
    const pending = response.conversations[3];
    const terminalFailure = response.conversations[1];

    expect(running?.recentInvocations[0]).toMatchObject({
      status: "running",
      firstTokenMs: 740,
      tUpstreamTtfbMs: null,
      tTotalMs: null,
    });
    expect(running?.lastInFlightAt).toBe("2026-04-06T12:00:00.000Z");
    expect(pending?.recentInvocations[0]).toMatchObject({
      status: "pending",
      tTotalMs: null,
    });
    expect(terminalFailure?.recentInvocations[0]).toMatchObject({
      status: "http_502",
      failureClass: "service_failure",
      failureKind: "upstream_timeout",
      errorMessage: "upstream gateway closed before first byte",
    });
    expect(terminalFailure?.lastTerminalAt).toBe("2026-04-06T11:59:10.000Z");
  });

  it("supports first-byte timing options and model-specific layout data", () => {
    const response = buildWorkingConversationsResponse({
      currentFirstTokenMs: null,
      currentTtfbMs: 91,
      gpt56Current: true,
    });
    const current = response.conversations[0]?.recentInvocations[0];

    expect(current).toMatchObject({
      status: "running",
      model: "gpt-5.6-sol",
      reasoningEffort: "max",
      firstTokenMs: null,
      tUpstreamTtfbMs: 91,
    });
  });

  it("represents conversations with missing history without inventing terminal or live times", () => {
    const conversation = createConversation("wc-empty", []);

    expect(conversation).toMatchObject({
      requestCount: 0,
      totalTokens: 0,
      totalCost: 0,
      lastTerminalAt: null,
      lastInFlightAt: null,
      recentInvocations: [],
      last24hRequests: [],
    });
  });

  it("preserves error and success account activity data", () => {
    const response = buildDashboardActivityResponse();
    const errorAccount = response.accounts[0];
    const successAccount = response.accounts[1];

    expect(errorAccount).toMatchObject({
      displayName: "CIII",
      requestCount: 12,
      successCount: 3,
      failureCount: 9,
    });
    expect(errorAccount?.recentInvocations[0]).toMatchObject({
      status: "http_429",
      failureKind: "upstream_rate_limit",
      upstreamAccountName: "CIII",
    });
    expect(errorAccount?.recentInvocations[0]?.errorMessage).toContain("DAILY_LIMIT_EXCEEDED");
    expect(successAccount?.recentInvocations[0]).toMatchObject({
      status: "success",
      failureClass: "none",
      upstreamAccountName: "dzw",
    });
    expect(response.summary.stats).toEqual(buildSummary("today"));
  });
});

describe("Dashboard summary and timeseries fixtures", () => {
  it("builds summary data for today, one day, and the default long range", () => {
    expect(buildSummary("today")).toMatchObject({ totalCount: 12_474, failureCount: 2_525 });
    expect(buildSummary("1d")).toMatchObject({ totalCount: 13_564, failureCount: 2_616 });
    expect(buildSummary("7d")).toMatchObject({ totalCount: 76_421, failureCount: 6_306 });
  });

  it("matches each timeseries range to its fixture bucket size", () => {
    expect(buildTimeseries("90d")).toMatchObject({
      effectiveBucket: "1d",
      points: expect.arrayContaining([
        expect.objectContaining({ bucketStart: expect.any(String) }),
      ]),
    });
    expect(buildTimeseries("90d").points).toHaveLength(90);
    expect(buildTimeseries("7d").points).toHaveLength(168);
    expect(buildTimeseries(null).points).toHaveLength(1_440);
  });
});

describe("createPreview", () => {
  it("preserves explicitly null timing values for provisional calls", () => {
    expect(
      createPreview({
        id: 17,
        invokeId: "wc-provisional",
        occurredAt: "2026-04-06T12:00:00.000Z",
        status: "running",
        firstTokenMs: null,
        tUpstreamTtfbMs: null,
        tUpstreamStreamMs: null,
        tTotalMs: null,
      }),
    ).toMatchObject({
      firstTokenMs: null,
      tUpstreamTtfbMs: null,
      tUpstreamStreamMs: null,
      tTotalMs: null,
    });
  });
});
