import { type ComponentProps, type ReactNode, useEffect, useRef, useState } from "react";
import { useLocation } from "react-router-dom";
import { useUpstreamAccountDetailRoute } from "../../hooks/useUpstreamAccountDetailRoute";
import type {
  ApiInvocation,
  ApiPoolUpstreamRequestAttempt,
  UpstreamAccountDetail,
  UpstreamAccountSummary,
} from "../../lib/api";
import { invocationStableKey } from "../../lib/invocation";
import { SharedUpstreamAccountDetailDrawer } from "../../pages/account-pool/UpstreamAccounts";
import {
  createStoryForwardProxyBindingNodes,
  formatStoryAttemptId,
} from "../records/invocationRecordsStoryFixtures";
import { InvocationTable } from "./InvocationTable";

const baseOccurredAt = "2026-02-25T10:15:30Z";
const LONG_PROXY_NAME = "ivan-hkl-vless-vision-01KFXRNYWYXKN4JHCF3CCV78GD";

const records: ApiInvocation[] = [
  {
    id: 1001,
    invokeId: "inv_01JSX0PQ3Z8CFQ7AJK8XEH2N4D",
    occurredAt: baseOccurredAt,
    createdAt: baseOccurredAt,
    source: "proxy",
    routeMode: "pool",
    transport: "websocket",
    upstreamAccountId: 21,
    upstreamAccountName: "Codex Team Alpha",
    proxyDisplayName: "Tokyo-Edge-1",
    responseContentEncoding: "gzip, br",
    requestCompressionAlgorithm: "zstd",
    endpoint: "/v1/responses",
    model: "gpt-5-mini",
    status: "success",
    inputTokens: 1632,
    outputTokens: 298,
    cacheInputTokens: 1240,
    reasoningTokens: 84,
    reasoningEffort: "high",
    totalTokens: 1930,
    cost: 0.0037,
    requesterIp: "203.0.113.42",
    promptCacheKey: "pck_6f35b9b20f0348af",
    requestedServiceTier: "priority",
    serviceTier: "priority",
    billingServiceTier: "priority",
    proxyWeightDelta: 0.55,
    tReqReadMs: 1.8,
    tReqParseMs: 3.2,
    tUpstreamConnectMs: 26.1,
    tUpstreamTtfbMs: 184.7,
    firstTokenMs: 742.6,
    tUpstreamStreamMs: 641.9,
    tRespParseMs: 8.6,
    tPersistMs: 2.1,
    tTotalMs: 870.4,
    priceVersion: "2026-02",
  },
  {
    id: 1002,
    invokeId: "inv_01JSX0Q6YHBFTDVMC3N5NF13R7",
    occurredAt: "2026-02-25T10:18:11Z",
    createdAt: "2026-02-25T10:18:11Z",
    source: "proxy",
    routeMode: "forward_proxy",
    proxyDisplayName: LONG_PROXY_NAME,
    responseContentEncoding: "identity",
    endpoint: "/v1/chat/completions",
    model: "gpt-5",
    status: "failed",
    inputTokens: 884,
    outputTokens: 0,
    cacheInputTokens: 0,
    reasoningEffort: "medium",
    totalTokens: 884,
    errorMessage: "upstream timeout while waiting first byte",
    failureKind: "upstream_timeout",
    requestedServiceTier: "priority",
    serviceTier: "auto",
    proxyWeightDelta: -0.68,
    tReqReadMs: 1.1,
    tReqParseMs: 2.3,
    tUpstreamConnectMs: 48.5,
    tUpstreamTtfbMs: null,
    tUpstreamStreamMs: null,
    tRespParseMs: null,
    tPersistMs: 1.9,
    tTotalMs: 30015.7,
  },
  {
    id: 1003,
    invokeId: "inv_01JSX0R9N0F2V8G54T5PG17WQH",
    occurredAt: "2026-02-25T10:22:48Z",
    createdAt: "2026-02-25T10:22:48Z",
    source: "proxy",
    routeMode: "pool",
    upstreamAccountId: 22,
    upstreamAccountName: "Codex Team Beta",
    proxyDisplayName: "Seoul-Edge-2",
    responseContentEncoding: "br",
    endpoint: "/v1/responses",
    model: "gpt-5.4",
    status: "success",
    inputTokens: 1520,
    outputTokens: 212,
    cacheInputTokens: 740,
    totalTokens: 1732,
    cost: 0.0051,
    requesterIp: "203.0.113.77",
    promptCacheKey: "pck_82c89c811a",
    requestedServiceTier: "priority",
    proxyWeightDelta: 0,
    tReqReadMs: 1.4,
    tReqParseMs: 2.8,
    tUpstreamConnectMs: 31.2,
    tUpstreamTtfbMs: 166.1,
    tUpstreamStreamMs: 512.4,
    tRespParseMs: 5.6,
    tPersistMs: 1.8,
    tTotalMs: 721.3,
  },
];

function requireFixture<T>(value: T | undefined): T {
  if (value === undefined) throw new Error("missing story fixture");
  return value;
}

const inFlightResponseDurationUnavailableRecords: ApiInvocation[] = [
  {
    ...records[0],
    id: -1004,
    invokeId: "inv_storybook_inflight_response_duration_unavailable",
    status: "running",
    firstTokenMs: 742.6,
    tUpstreamStreamMs: null,
  },
];

const compactLatencyFormattingRecords: ApiInvocation[] = [
  {
    ...requireFixture(records[0]),
    id: -1005,
    invokeId: "inv_storybook_compact_latency_decimal",
    firstTokenMs: 1_234,
    tUpstreamStreamMs: 7_890,
  },
  {
    ...requireFixture(records[0]),
    id: -1006,
    invokeId: "inv_storybook_compact_latency_integer",
    occurredAt: "2026-02-25T10:30:01Z",
    createdAt: "2026-02-25T10:30:01Z",
    firstTokenMs: 100_040,
    tUpstreamStreamMs: 123_460,
  },
];

const modelRoutingMismatchRecords: ApiInvocation[] = [
  {
    ...records[0],
    id: 1011,
    invokeId: "inv_01JSX0PQ3Z8CFQ7AJK8XEH2N4E",
    model: "gpt-5.5",
    requestModel: "gpt-5.4",
    responseModel: "gpt-5.5",
  },
  {
    ...records[1],
    id: 1012,
    invokeId: "inv_01JSX0Q6YHBFTDVMC3N5NF13R8",
    requestModel: "gpt-5",
    responseModel: "gpt-5",
  },
];

const legacyModelOnlyRecords: ApiInvocation[] = [
  {
    ...records[2],
    id: 1013,
    invokeId: "inv_01JSX0R9N0F2V8G54T5PG17WQI",
    model: "gpt-5.4",
    requestModel: undefined,
    responseModel: undefined,
  },
];

const poolRoutingAccountStateRecords: ApiInvocation[] = [
  {
    ...records[0],
    id: 1020,
    invokeId: "inv_pool_routing_account_named",
    occurredAt: "2026-02-25T10:24:00Z",
    createdAt: "2026-02-25T10:24:00Z",
    routeMode: "pool",
    upstreamAccountId: 21,
    upstreamAccountName: "Codex Team Alpha",
    status: "running",
    totalTokens: 0,
    cost: undefined,
    tTotalMs: null,
  },
  {
    ...records[0],
    id: 1021,
    invokeId: "inv_pool_routing_account_missing",
    occurredAt: "2026-02-25T10:23:00Z",
    createdAt: "2026-02-25T10:23:00Z",
    routeMode: "pool",
    upstreamAccountId: undefined,
    upstreamAccountName: undefined,
    status: "pending",
    totalTokens: 0,
    cost: undefined,
    tTotalMs: null,
  },
  {
    ...records[2],
    id: 1022,
    invokeId: "inv_pool_routing_account_terminal",
    occurredAt: "2026-02-25T10:22:00Z",
    createdAt: "2026-02-25T10:22:00Z",
    routeMode: "pool",
    upstreamAccountId: 22,
    upstreamAccountName: "Codex Team Beta",
    status: "success",
  },
];

const missingWindowDrawerRecords: ApiInvocation[] = [
  {
    id: 1023,
    invokeId: "inv_storybook_missing_window_drawer",
    occurredAt: "2026-03-16T10:25:00Z",
    createdAt: "2026-03-16T10:25:00Z",
    source: "proxy",
    routeMode: "pool",
    upstreamAccountId: 23,
    upstreamAccountName: "Team key - missing weekly limit",
    proxyDisplayName: "Tokyo-Edge-Quota-Mock",
    responseContentEncoding: "gzip",
    endpoint: "/v1/responses",
    model: "gpt-5.4",
    status: "success",
    inputTokens: 1410,
    outputTokens: 188,
    totalTokens: 1598,
    cost: 0.0042,
    tUpstreamTtfbMs: 121.4,
    tTotalMs: 688.3,
  },
];

const fastIndicatorRecords: ApiInvocation[] = [
  {
    id: 1101,
    invokeId: "inv_fast_effective",
    occurredAt: "2026-02-25T10:30:00Z",
    createdAt: "2026-02-25T10:30:00Z",
    source: "proxy",
    proxyDisplayName: "Fast-effective",
    endpoint: "/v1/responses",
    model: "gpt-5-mini",
    status: "success",
    requestedServiceTier: "priority",
    serviceTier: "priority",
    billingServiceTier: "priority",
    inputTokens: 1200,
    outputTokens: 240,
    totalTokens: 1440,
    cost: 0.0032,
    tUpstreamTtfbMs: 118.3,
    tTotalMs: 640.2,
  },
  {
    id: 1102,
    invokeId: "inv_fast_requested_auto",
    occurredAt: "2026-02-25T10:31:00Z",
    createdAt: "2026-02-25T10:31:00Z",
    source: "proxy",
    routeMode: "pool",
    upstreamAccountId: 2568,
    upstreamAccountName: "API Keys Pool",
    proxyDisplayName: "API Keys requested-tier priority",
    endpoint: "/v1/responses",
    model: "gpt-5",
    status: "failed",
    requestedServiceTier: "priority",
    serviceTier: "default",
    billingServiceTier: "priority",
    priceVersion: "openai-standard-2026-02-23@requested-tier",
    inputTokens: 980,
    outputTokens: 0,
    totalTokens: 980,
    errorMessage: "upstream timeout while waiting first byte",
    tUpstreamTtfbMs: null,
    tTotalMs: 30010.5,
  },
  {
    id: 1103,
    invokeId: "inv_fast_requested_missing",
    occurredAt: "2026-02-25T10:32:00Z",
    createdAt: "2026-02-25T10:32:00Z",
    source: "proxy",
    proxyDisplayName: "Fast-requested-missing",
    endpoint: "/v1/responses",
    model: "gpt-5.4",
    status: "success",
    requestedServiceTier: "priority",
    inputTokens: 1024,
    outputTokens: 196,
    totalTokens: 1220,
    cost: 0.0038,
    tUpstreamTtfbMs: 142.6,
    tTotalMs: 702.1,
  },
  {
    id: 1104,
    invokeId: "inv_fast_effective_auto_request",
    occurredAt: "2026-02-25T10:33:00Z",
    createdAt: "2026-02-25T10:33:00Z",
    source: "proxy",
    proxyDisplayName: "Fast-effective-auto-request",
    endpoint: "/v1/responses",
    model: "gpt-5.4",
    status: "success",
    requestedServiceTier: "auto",
    serviceTier: "priority",
    billingServiceTier: "priority",
    inputTokens: 1188,
    outputTokens: 202,
    totalTokens: 1390,
    cost: 0.0041,
    tUpstreamTtfbMs: 104.4,
    tTotalMs: 611.9,
  },
  {
    id: 1105,
    invokeId: "inv_fast_none_flex",
    occurredAt: "2026-02-25T10:34:00Z",
    createdAt: "2026-02-25T10:34:00Z",
    source: "proxy",
    proxyDisplayName: "Fast-none-flex",
    endpoint: "/v1/responses",
    model: "gpt-5.4",
    status: "success",
    requestedServiceTier: "flex",
    serviceTier: "flex",
    inputTokens: 1160,
    outputTokens: 188,
    totalTokens: 1348,
    cost: 0.0035,
    tUpstreamTtfbMs: 156.8,
    tTotalMs: 734.7,
  },
];

const endpointBadgeRecords: ApiInvocation[] = [
  {
    id: 1501,
    invokeId: "inv_endpoint_badge_responses",
    occurredAt: "2026-02-25T10:36:00Z",
    createdAt: "2026-02-25T10:36:00Z",
    source: "proxy",
    proxyDisplayName: "Endpoint-responses",
    endpoint: "/v1/responses",
    model: "gpt-5.4",
    status: "success",
    totalTokens: 1234,
    cost: 0.0031,
    tUpstreamTtfbMs: 108.6,
    tTotalMs: 602.4,
  },
  {
    id: 1502,
    invokeId: "inv_endpoint_badge_chat",
    occurredAt: "2026-02-25T10:37:00Z",
    createdAt: "2026-02-25T10:37:00Z",
    source: "proxy",
    proxyDisplayName: "Endpoint-chat",
    endpoint: "/v1/chat/completions",
    model: "gpt-5",
    status: "failed",
    totalTokens: 888,
    cost: 0.0024,
    errorMessage: "upstream timeout while waiting first byte",
    tUpstreamTtfbMs: null,
    tTotalMs: 30004.8,
  },
  {
    id: 1503,
    invokeId: "inv_endpoint_badge_compact",
    occurredAt: "2026-02-25T10:38:00Z",
    createdAt: "2026-02-25T10:38:00Z",
    source: "proxy",
    proxyDisplayName: "Endpoint-compact",
    endpoint: "/v1/responses/compact",
    model: "gpt-5.4",
    status: "success",
    totalTokens: 640,
    cost: 0.0017,
    tUpstreamTtfbMs: 92.2,
    tTotalMs: 511.3,
  },
  {
    id: 1504,
    invokeId: "inv_endpoint_badge_raw",
    occurredAt: "2026-02-25T10:39:00Z",
    createdAt: "2026-02-25T10:39:00Z",
    source: "proxy",
    proxyDisplayName: "Endpoint-raw",
    endpoint: `/v1/responses/${"very-long-segment-".repeat(5)}`,
    model: "gpt-5.4-mini",
    status: "success",
    totalTokens: 420,
    cost: 0.0011,
    tUpstreamTtfbMs: 132.8,
    tTotalMs: 744.6,
  },
  {
    id: 1505,
    invokeId: "inv_endpoint_badge_remote_v2_running",
    occurredAt: "2026-02-25T10:40:00Z",
    createdAt: "2026-02-25T10:40:00Z",
    source: "proxy",
    proxyDisplayName: "Endpoint-remote-v2",
    endpoint: "/v1/responses",
    compactionRequestKind: "remote_v2",
    imageIntent: "yes",
    model: "gpt-5.4",
    status: "running",
    livePhase: "requesting",
    totalTokens: 512,
    requestedServiceTier: "priority",
  },
  {
    id: 1506,
    invokeId: "inv_endpoint_badge_remote_v2_completed",
    occurredAt: "2026-02-25T10:41:00Z",
    createdAt: "2026-02-25T10:41:00Z",
    source: "proxy",
    proxyDisplayName: "Endpoint-remote-v2-completed",
    endpoint: "/v1/responses",
    compactionRequestKind: "remote_v2",
    compactionResponseKind: "remote_v2",
    imageIntent: "direct_image",
    model: "gpt-5.4",
    status: "success",
    totalTokens: 598,
    cost: 0.0019,
    tUpstreamTtfbMs: 116.2,
    tTotalMs: 684.9,
  },
  {
    id: 1507,
    invokeId: "inv_endpoint_badge_image_generation",
    occurredAt: "2026-02-25T10:42:00Z",
    createdAt: "2026-02-25T10:42:00Z",
    source: "proxy",
    proxyDisplayName: "Endpoint-image-gen",
    endpoint: "/v1/images/generations",
    imageIntent: "yes",
    model: "gpt-image-1",
    status: "success",
    totalTokens: 420,
    cost: 0.0022,
    tUpstreamTtfbMs: 104.4,
    tTotalMs: 621.3,
  },
  {
    id: 1508,
    invokeId: "inv_endpoint_badge_image_edit",
    occurredAt: "2026-02-25T10:43:00Z",
    createdAt: "2026-02-25T10:43:00Z",
    source: "proxy",
    proxyDisplayName: "Endpoint-image-edit",
    endpoint: "/v1/images/edits",
    imageIntent: "direct_image",
    model: "gpt-image-1",
    status: "success",
    totalTokens: 386,
    cost: 0.002,
    tUpstreamTtfbMs: 126.1,
    tTotalMs: 702.5,
  },
  {
    id: 1509,
    invokeId: "inv_endpoint_badge_image_generic",
    occurredAt: "2026-02-25T10:44:00Z",
    createdAt: "2026-02-25T10:44:00Z",
    source: "proxy",
    proxyDisplayName: "Endpoint-image-generic",
    endpoint: "/v1/images/variations",
    model: "gpt-image-1",
    status: "success",
    totalTokens: 344,
    cost: 0.0018,
    tUpstreamTtfbMs: 112.7,
    tTotalMs: 648.2,
  },
];

const reasoningEffortRecords: ApiInvocation[] = [
  {
    id: 2001,
    invokeId: "inv_reasoning_none",
    occurredAt: "2026-02-25T11:00:00Z",
    createdAt: "2026-02-25T11:00:00Z",
    source: "proxy",
    proxyDisplayName: "Reasoning-none",
    endpoint: "/v1/chat/completions",
    model: "gpt-5.1",
    status: "success",
    inputTokens: 640,
    outputTokens: 112,
    cacheInputTokens: 0,
    reasoningEffort: "none",
    reasoningTokens: 0,
    totalTokens: 752,
    cost: 0.0018,
    tUpstreamTtfbMs: 96.4,
    tTotalMs: 411.7,
  },
  {
    id: 2002,
    invokeId: "inv_reasoning_minimal",
    occurredAt: "2026-02-25T11:02:00Z",
    createdAt: "2026-02-25T11:02:00Z",
    source: "proxy",
    proxyDisplayName: "Reasoning-minimal",
    endpoint: "/v1/responses",
    model: "gpt-5",
    status: "success",
    inputTokens: 712,
    outputTokens: 144,
    cacheInputTokens: 128,
    reasoningEffort: "minimal",
    reasoningTokens: 12,
    totalTokens: 856,
    cost: 0.0021,
    tUpstreamTtfbMs: 118.1,
    tTotalMs: 588.2,
  },
  {
    id: 2003,
    invokeId: "inv_reasoning_low",
    occurredAt: "2026-02-25T11:04:00Z",
    createdAt: "2026-02-25T11:04:00Z",
    source: "proxy",
    proxyDisplayName: "Reasoning-low",
    endpoint: "/v1/responses",
    model: "gpt-5-mini",
    status: "success",
    inputTokens: 804,
    outputTokens: 166,
    cacheInputTokens: 256,
    reasoningEffort: "low",
    reasoningTokens: 28,
    totalTokens: 970,
    cost: 0.0024,
    tUpstreamTtfbMs: 132.5,
    tTotalMs: 710.4,
  },
  {
    id: 2004,
    invokeId: "inv_reasoning_medium",
    occurredAt: "2026-02-25T11:06:00Z",
    createdAt: "2026-02-25T11:06:00Z",
    source: "proxy",
    proxyDisplayName: "Reasoning-medium",
    endpoint: "/v1/chat/completions",
    model: "gpt-5",
    status: "failed",
    inputTokens: 920,
    outputTokens: 0,
    cacheInputTokens: 0,
    reasoningEffort: "medium",
    totalTokens: 920,
    errorMessage: "upstream timeout while waiting first byte",
    failureKind: "upstream_timeout",
    tUpstreamTtfbMs: null,
    tTotalMs: 30012.0,
  },
  {
    id: 2005,
    invokeId: "inv_reasoning_high",
    occurredAt: "2026-02-25T11:08:00Z",
    createdAt: "2026-02-25T11:08:00Z",
    source: "proxy",
    proxyDisplayName: "Reasoning-high",
    endpoint: "/v1/responses",
    model: "gpt-5",
    status: "success",
    inputTokens: 1012,
    outputTokens: 244,
    cacheInputTokens: 320,
    reasoningEffort: "high",
    reasoningTokens: 84,
    totalTokens: 1256,
    cost: 0.0031,
    tUpstreamTtfbMs: 188.4,
    tTotalMs: 962.6,
  },
  {
    id: 2006,
    invokeId: "inv_reasoning_xhigh",
    occurredAt: "2026-02-25T11:10:00Z",
    createdAt: "2026-02-25T11:10:00Z",
    source: "proxy",
    proxyDisplayName: "Reasoning-xhigh",
    endpoint: "/v1/responses",
    model: "gpt-5.2",
    status: "success",
    inputTokens: 1130,
    outputTokens: 318,
    cacheInputTokens: 512,
    reasoningEffort: "xhigh",
    reasoningTokens: 146,
    totalTokens: 1448,
    cost: 0.0048,
    tUpstreamTtfbMs: 261.3,
    tTotalMs: 1384.9,
  },
  {
    id: 2007,
    invokeId: "inv_reasoning_max",
    occurredAt: "2026-02-25T11:12:00Z",
    createdAt: "2026-02-25T11:12:00Z",
    source: "proxy",
    proxyDisplayName: "Reasoning-max",
    endpoint: "/v1/responses",
    model: "gpt-5.6-sol",
    status: "success",
    inputTokens: 1280,
    outputTokens: 360,
    cacheInputTokens: 576,
    reasoningEffort: " MAX ",
    reasoningTokens: 176,
    totalTokens: 1640,
    cost: 0.0054,
    tUpstreamTtfbMs: 288.5,
    tTotalMs: 1548.2,
  },
  {
    id: 2008,
    invokeId: "inv_reasoning_ultra",
    occurredAt: "2026-02-25T11:14:00Z",
    createdAt: "2026-02-25T11:14:00Z",
    source: "proxy",
    proxyDisplayName: "Reasoning-ultra",
    endpoint: "/v1/responses",
    model: "gpt-5.6-luna",
    status: "success",
    inputTokens: 1360,
    outputTokens: 412,
    cacheInputTokens: 640,
    reasoningEffort: "ULTRA",
    reasoningTokens: 238,
    totalTokens: 1772,
    cost: 0.0062,
    tUpstreamTtfbMs: 322.4,
    tTotalMs: 1788.3,
  },
  {
    id: 2009,
    invokeId: "inv_reasoning_missing",
    occurredAt: "2026-02-25T11:16:00Z",
    createdAt: "2026-02-25T11:16:00Z",
    source: "proxy",
    proxyDisplayName: "Reasoning-missing",
    endpoint: "/v1/responses",
    model: "gpt-5-mini",
    status: "success",
    inputTokens: 540,
    outputTokens: 90,
    cacheInputTokens: 64,
    totalTokens: 630,
    cost: 0.0015,
    tUpstreamTtfbMs: 104.7,
    tTotalMs: 498.5,
  },
  {
    id: 2010,
    invokeId: "inv_reasoning_unknown",
    occurredAt: "2026-02-25T11:18:00Z",
    createdAt: "2026-02-25T11:18:00Z",
    source: "proxy",
    proxyDisplayName: "Reasoning-unknown",
    endpoint: "/v1/responses",
    model: "custom-reasoning-model",
    status: "success",
    inputTokens: 600,
    outputTokens: 120,
    cacheInputTokens: 0,
    reasoningEffort: "custom-tier",
    reasoningTokens: 33,
    totalTokens: 720,
    cost: 0.0019,
    tUpstreamTtfbMs: 124.2,
    tTotalMs: 544.0,
  },
];

const accountDetails = new Map<number, UpstreamAccountDetail>([
  [
    2,
    {
      id: 2,
      kind: "oauth_codex",
      provider: "openai",
      displayName: "NSNGC",
      groupName: "nsngc",
      isMother: false,
      status: "active",
      enabled: true,
      email: "nsngc@example.com",
      chatgptAccountId: "org_nsngc",
      chatgptUserId: "user_nsngc",
      planType: "team",
      maskedApiKey: null,
      lastSyncedAt: "2026-03-16T09:12:00Z",
      lastSuccessfulSyncAt: "2026-03-16T09:11:00Z",
      lastError: null,
      lastErrorAt: null,
      tokenExpiresAt: "2026-03-16T12:15:00Z",
      lastRefreshedAt: "2026-03-16T09:11:30Z",
      primaryWindow: {
        usedPercent: 18,
        usedText: "18 / 100",
        limitText: "100 requests",
        resetsAt: "2026-03-16T10:00:00Z",
        windowDurationMins: 300,
      },
      secondaryWindow: {
        usedPercent: 31,
        usedText: "31 / 100",
        limitText: "100 requests",
        resetsAt: "2026-03-17T00:00:00Z",
        windowDurationMins: 10080,
      },
      credits: null,
      localLimits: null,
      duplicateInfo: null,
      tags: [],
      effectiveRoutingRule: {
        allowCutOut: true,
        allowCutIn: true,
        sourceTagIds: [],
        sourceTagNames: [],
      },
      note: null,
      upstreamBaseUrl: "https://claude-relay-service.nsngc.org",
      history: [],
    },
  ],
  [
    9,
    {
      id: 9,
      kind: "oauth_codex",
      provider: "openai",
      displayName: "Fallback Account 9",
      groupName: "fallback",
      isMother: false,
      status: "active",
      enabled: true,
      email: "fallback9@example.com",
      chatgptAccountId: "org_fallback_9",
      chatgptUserId: "user_fallback_9",
      planType: "pro",
      maskedApiKey: null,
      lastSyncedAt: "2026-03-16T07:12:00Z",
      lastSuccessfulSyncAt: "2026-03-16T07:11:00Z",
      lastError: null,
      lastErrorAt: null,
      tokenExpiresAt: "2026-03-16T11:45:00Z",
      lastRefreshedAt: "2026-03-16T07:11:30Z",
      primaryWindow: {
        usedPercent: 9,
        usedText: "9 / 100",
        limitText: "100 requests",
        resetsAt: "2026-03-16T10:00:00Z",
        windowDurationMins: 300,
      },
      secondaryWindow: {
        usedPercent: 14,
        usedText: "14 / 100",
        limitText: "100 requests",
        resetsAt: "2026-03-17T00:00:00Z",
        windowDurationMins: 10080,
      },
      credits: null,
      localLimits: null,
      duplicateInfo: null,
      tags: [],
      effectiveRoutingRule: {
        allowCutOut: true,
        allowCutIn: true,
        sourceTagIds: [],
        sourceTagNames: [],
      },
      note: null,
      upstreamBaseUrl: null,
      history: [],
    },
  ],
  [
    21,
    {
      id: 21,
      kind: "oauth_codex",
      provider: "openai",
      displayName: "Codex Team Alpha",
      groupName: "team-alpha",
      isMother: true,
      status: "active",
      enabled: true,
      email: "alpha@example.com",
      chatgptAccountId: "org_alpha",
      chatgptUserId: "user_alpha",
      planType: "team",
      maskedApiKey: null,
      lastSyncedAt: "2026-03-16T09:10:00Z",
      lastSuccessfulSyncAt: "2026-03-16T09:08:00Z",
      lastError:
        "Two upstream 429 responses were observed during the latest compact capability probe.",
      lastErrorAt: "2026-03-16T09:11:30Z",
      tokenExpiresAt: "2026-03-16T12:00:00Z",
      lastRefreshedAt: "2026-03-16T09:09:00Z",
      primaryWindow: {
        usedPercent: 22,
        usedText: "22 / 100",
        limitText: "100 requests",
        resetsAt: "2026-03-16T10:00:00Z",
        windowDurationMins: 300,
      },
      secondaryWindow: {
        usedPercent: 36,
        usedText: "36 / 100",
        limitText: "100 requests",
        resetsAt: "2026-03-17T00:00:00Z",
        windowDurationMins: 10080,
      },
      credits: {
        hasCredits: true,
        unlimited: false,
        balance: "42.7",
      },
      localLimits: null,
      duplicateInfo: null,
      tags: [],
      effectiveRoutingRule: {
        allowCutOut: true,
        allowCutIn: true,
        sourceTagIds: [],
        sourceTagNames: [],
      },
      note: null,
      upstreamBaseUrl: null,
      history: [
        {
          capturedAt: "2026-03-15T06:00:00Z",
          primaryUsedPercent: 14,
          secondaryUsedPercent: 22,
          creditsBalance: "46.2",
        },
        {
          capturedAt: "2026-03-15T12:00:00Z",
          primaryUsedPercent: 18,
          secondaryUsedPercent: 27,
          creditsBalance: "45.8",
        },
        {
          capturedAt: "2026-03-15T18:00:00Z",
          primaryUsedPercent: 24,
          secondaryUsedPercent: 31,
          creditsBalance: "45.1",
        },
        {
          capturedAt: "2026-03-16T00:00:00Z",
          primaryUsedPercent: 30,
          secondaryUsedPercent: 34,
          creditsBalance: "44.6",
        },
        {
          capturedAt: "2026-03-16T06:00:00Z",
          primaryUsedPercent: 26,
          secondaryUsedPercent: 35,
          creditsBalance: "43.9",
        },
        {
          capturedAt: "2026-03-16T09:00:00Z",
          primaryUsedPercent: 22,
          secondaryUsedPercent: 36,
          creditsBalance: "42.7",
        },
      ],
    },
  ],
  [
    22,
    {
      id: 22,
      kind: "oauth_codex",
      provider: "openai",
      displayName: "Codex Team Beta",
      groupName: "team-beta",
      isMother: false,
      status: "active",
      enabled: true,
      email: "beta@example.com",
      chatgptAccountId: "org_beta",
      chatgptUserId: "user_beta",
      planType: "pro",
      maskedApiKey: null,
      lastSyncedAt: "2026-03-16T08:20:00Z",
      lastSuccessfulSyncAt: "2026-03-16T08:19:00Z",
      lastError: null,
      lastErrorAt: null,
      tokenExpiresAt: "2026-03-16T11:50:00Z",
      lastRefreshedAt: "2026-03-16T08:19:30Z",
      primaryWindow: {
        usedPercent: 48,
        usedText: "48 / 100",
        limitText: "100 requests",
        resetsAt: "2026-03-16T10:00:00Z",
        windowDurationMins: 300,
      },
      secondaryWindow: {
        usedPercent: 52,
        usedText: "52 / 100",
        limitText: "100 requests",
        resetsAt: "2026-03-17T00:00:00Z",
        windowDurationMins: 10080,
      },
      credits: null,
      localLimits: null,
      duplicateInfo: null,
      tags: [],
      effectiveRoutingRule: {
        allowCutOut: true,
        allowCutIn: true,
        sourceTagIds: [],
        sourceTagNames: [],
      },
      note: null,
      upstreamBaseUrl: null,
      history: [],
    },
  ],
  [
    23,
    {
      id: 23,
      kind: "api_key",
      provider: "openai",
      displayName: "Team key - missing weekly limit",
      groupName: "quota-fallback",
      isMother: false,
      status: "active",
      enabled: true,
      email: null,
      chatgptAccountId: null,
      chatgptUserId: null,
      planType: "team",
      maskedApiKey: "sk-live••••••missing",
      lastSyncedAt: "2026-03-16T10:20:00Z",
      lastSuccessfulSyncAt: "2026-03-16T10:19:00Z",
      lastError: null,
      lastErrorAt: null,
      tokenExpiresAt: null,
      lastRefreshedAt: "2026-03-16T10:19:30Z",
      primaryWindow: {
        usedPercent: 18,
        usedText: "18 requests",
        limitText: "120 requests",
        resetsAt: "2026-03-16T13:00:00Z",
        windowDurationMins: 300,
      },
      secondaryWindow: null,
      credits: null,
      localLimits: {
        primaryLimit: 120,
        secondaryLimit: null,
        limitUnit: "requests",
      },
      duplicateInfo: null,
      tags: [],
      effectiveRoutingRule: {
        allowCutOut: true,
        allowCutIn: true,
        sourceTagIds: [],
        sourceTagNames: [],
      },
      note: "Secondary quota window is intentionally missing in this story.",
      upstreamBaseUrl: null,
      history: [
        {
          capturedAt: "2026-03-16T04:00:00Z",
          primaryUsedPercent: 12,
          secondaryUsedPercent: null,
          creditsBalance: null,
        },
        {
          capturedAt: "2026-03-16T08:00:00Z",
          primaryUsedPercent: 15,
          secondaryUsedPercent: null,
          creditsBalance: null,
        },
        {
          capturedAt: "2026-03-16T10:00:00Z",
          primaryUsedPercent: 18,
          secondaryUsedPercent: null,
          creditsBalance: null,
        },
      ],
    },
  ],
]);

function jsonResponse(body: unknown, status = 200) {
  return Promise.resolve(
    new Response(JSON.stringify(body), {
      status,
      headers: {
        "Content-Type": "application/json",
      },
    }),
  );
}

function buildAccountSummary(detail: UpstreamAccountDetail): UpstreamAccountSummary {
  return {
    id: detail.id,
    kind: detail.kind,
    provider: detail.provider,
    displayName: detail.displayName,
    groupName: detail.groupName,
    isMother: detail.isMother,
    status: detail.status,
    enabled: detail.enabled,
    email: detail.email,
    chatgptAccountId: detail.chatgptAccountId,
    planType: detail.planType,
    maskedApiKey: detail.maskedApiKey,
    lastSyncedAt: detail.lastSyncedAt,
    lastSuccessfulSyncAt: detail.lastSuccessfulSyncAt,
    lastError: detail.lastError,
    lastErrorAt: detail.lastErrorAt,
    tokenExpiresAt: detail.tokenExpiresAt,
    primaryWindow: detail.primaryWindow,
    secondaryWindow: detail.secondaryWindow,
    credits: detail.credits,
    localLimits: detail.localLimits,
    duplicateInfo: detail.duplicateInfo,
    tags: detail.tags,
    effectiveRoutingRule: detail.effectiveRoutingRule,
  };
}

function buildStickyConversations(accountId: number) {
  return {
    rangeStart: "2026-03-16T00:00:00Z",
    rangeEnd: "2026-03-17T00:00:00Z",
    conversations:
      accountId === 21
        ? [
            {
              stickyKey: "019ce3a1-6787-7910-b0fd-c246d6f6a901",
              requestCount: 10,
              totalTokens: 455170,
              totalCost: 0.3507,
              createdAt: "2026-03-16T04:01:20.000Z",
              lastActivityAt: "2026-03-16T04:03:02.000Z",
              last24hRequests: [
                {
                  occurredAt: "2026-03-16T10:15:00.000Z",
                  status: "success",
                  isSuccess: true,
                  requestTokens: 102440,
                  cumulativeTokens: 102440,
                },
                {
                  occurredAt: "2026-03-16T18:20:00.000Z",
                  status: "success",
                  isSuccess: true,
                  requestTokens: 154380,
                  cumulativeTokens: 256820,
                },
              ],
            },
          ]
        : [],
  };
}

function StorybookInvocationTableMock({ children }: { children: ReactNode }) {
  const originalFetchRef = useRef<typeof window.fetch | null>(null);

  if (typeof window !== "undefined" && originalFetchRef.current == null) {
    originalFetchRef.current = window.fetch.bind(window);
    window.fetch = async (input, init) => {
      const request =
        typeof input === "string" ? input : input instanceof URL ? input.toString() : input.url;
      const method =
        init?.method ?? (typeof input === "string" || input instanceof URL ? "GET" : input.method);

      if (method.toUpperCase() === "GET") {
        const url = new URL(request, window.location.origin);
        if (url.pathname === "/api/pool/upstream-accounts") {
          const items = Array.from(accountDetails.values()).map(buildAccountSummary);
          return jsonResponse({
            writesEnabled: true,
            items,
            groups: items
              .map((item) => item.groupName?.trim())
              .filter((value): value is string => Boolean(value))
              .sort()
              .map((groupName) => ({ groupName, note: null })),
            routing: {
              apiKeyConfigured: true,
              maskedApiKey: "pool-live••••••c0de",
            },
          });
        }
        if (url.pathname === "/api/pool/tags") {
          return jsonResponse({
            writesEnabled: true,
            items: [],
          });
        }
        if (url.pathname === "/api/pool/forward-proxy-binding-nodes") {
          return jsonResponse(createStoryForwardProxyBindingNodes(url.searchParams.getAll("key")));
        }
        const match = url.pathname.match(/^\/api\/pool\/upstream-accounts\/(\d+)$/);
        if (match) {
          const detail = accountDetails.get(Number(match[1]));
          if (detail) return jsonResponse(detail);
          return jsonResponse({ message: "Not found" }, 404);
        }
        const stickyMatch = url.pathname.match(
          /^\/api\/pool\/upstream-accounts\/(\d+)\/sticky-keys$/,
        );
        if (stickyMatch) {
          return jsonResponse(buildStickyConversations(Number(stickyMatch[1])));
        }
        const poolAttemptMatch = url.pathname.match(/^\/api\/invocations\/([^/]+)\/pool-attempts$/);
        if (poolAttemptMatch) {
          const resolver = storybookPoolAttemptResponses.get(
            decodeURIComponent(poolAttemptMatch[1]),
          );
          return jsonResponse(resolver ? resolver() : []);
        }
      }

      return originalFetchRef.current
        ? originalFetchRef.current(input as Parameters<typeof fetch>[0], init)
        : fetch(input as Parameters<typeof fetch>[0], init);
    };
  }

  useEffect(() => {
    return () => {
      storybookPoolAttemptResponses.clear();
      if (originalFetchRef.current) {
        window.fetch = originalFetchRef.current;
      }
    };
  }, []);

  return <>{children}</>;
}

function InvocationTableStoryShell({ children }: { children: ReactNode }) {
  return (
    <div className="bg-base-200 px-6 py-6 text-base-content">
      <div className="mx-auto w-full max-w-6xl p-6">
        <section className="card bg-base-100 shadow-sm">
          <div className="card-body gap-4 p-6">{children}</div>
        </section>
      </div>
    </div>
  );
}

function InvocationTableSharedDrawerPreview(props: ComponentProps<typeof InvocationTable>) {
  const location = useLocation();
  const { upstreamAccountId, openUpstreamAccount, closeUpstreamAccount } =
    useUpstreamAccountDetailRoute();

  return (
    <>
      <span className="sr-only" data-testid="invocation-route-state">
        {location.pathname}
        {location.search}
      </span>
      <InvocationTable
        {...props}
        onOpenUpstreamAccount={(accountId) => openUpstreamAccount(accountId)}
      />
      <SharedUpstreamAccountDetailDrawer
        open={upstreamAccountId != null}
        accountId={upstreamAccountId}
        onClose={closeUpstreamAccount}
      />
    </>
  );
}

function RunningInvocationLifecyclePreview() {
  const occurredAtRef = useRef<string>(new Date(Date.now() - 1200).toISOString());
  const [phase, setPhase] = useState<"initial" | "enriched" | "terminal">("initial");

  useEffect(() => {
    const enrichTimer = window.setTimeout(() => setPhase("enriched"), 1200);
    const terminalTimer = window.setTimeout(() => setPhase("terminal"), 2800);
    return () => {
      window.clearTimeout(enrichTimer);
      window.clearTimeout(terminalTimer);
    };
  }, []);

  const occurredAt = occurredAtRef.current;
  const terminalElapsedMs = Math.max(0, Date.now() - Date.parse(occurredAt));
  const lifecycleRecord: ApiInvocation =
    phase === "terminal"
      ? {
          id: 1201,
          invokeId: "inv_storybook_running_lifecycle",
          occurredAt,
          createdAt: occurredAt,
          source: "proxy",
          routeMode: "pool",
          upstreamAccountId: 21,
          upstreamAccountName: "Codex Team Alpha",
          proxyDisplayName: "Storybook Live Running Demo",
          responseContentEncoding: "gzip, br",
          endpoint: "/v1/responses/compact",
          model: "gpt-5.4",
          status: "success",
          inputTokens: 2048,
          outputTokens: 188,
          cacheInputTokens: 1536,
          reasoningTokens: 64,
          reasoningEffort: "high",
          totalTokens: 2236,
          cost: 0.0046,
          requestedServiceTier: "priority",
          serviceTier: "priority",
          billingServiceTier: "priority",
          proxyWeightDelta: 0.42,
          tUpstreamTtfbMs: 184.2,
          tTotalMs: Number(terminalElapsedMs.toFixed(1)),
        }
      : {
          id: -1201,
          invokeId: "inv_storybook_running_lifecycle",
          occurredAt,
          createdAt: occurredAt,
          source: "proxy",
          routeMode: "pool",
          upstreamAccountId: 21,
          upstreamAccountName: "Codex Team Alpha",
          proxyDisplayName: "Storybook Live Running Demo",
          endpoint: "/v1/responses/compact",
          model: "gpt-5.4",
          status: "running",
          inputTokens: 2048,
          cacheInputTokens: 1536,
          totalTokens: 2048,
          requestedServiceTier: "priority",
          responseContentEncoding: phase === "enriched" ? "gzip" : undefined,
          tUpstreamTtfbMs: phase === "enriched" ? 184.2 : null,
        };

  return <InvocationTable records={[lifecycleRecord]} isLoading={false} error={null} />;
}

function buildPoolAttemptLifecycleRecord(
  phase: "running" | "success",
  occurredAt: string,
): ApiInvocation {
  if (phase === "success") {
    return {
      id: 1401,
      invokeId: "inv_storybook_pool_attempt_detail_lifecycle",
      occurredAt,
      createdAt: occurredAt,
      source: "proxy",
      routeMode: "pool",
      upstreamAccountId: 21,
      upstreamAccountName: "Codex Team Alpha",
      proxyDisplayName: "Storybook Pool Attempt Lifecycle",
      endpoint: "/v1/responses",
      model: "gpt-5.4",
      status: "success",
      poolAttemptCount: 1,
      poolDistinctAccountCount: 1,
      inputTokens: 2048,
      outputTokens: 188,
      cacheInputTokens: 1536,
      totalTokens: 2236,
      cost: 0.0046,
      requestedServiceTier: "priority",
      serviceTier: "priority",
      billingServiceTier: "priority",
      responseContentEncoding: "gzip",
      tUpstreamConnectMs: 26.4,
      tUpstreamTtfbMs: 148.2,
      tUpstreamStreamMs: 612.8,
      tTotalMs: 804.7,
    };
  }

  return {
    id: -1401,
    invokeId: "inv_storybook_pool_attempt_detail_lifecycle",
    occurredAt,
    createdAt: occurredAt,
    source: "proxy",
    routeMode: "pool",
    upstreamAccountId: 21,
    upstreamAccountName: "Codex Team Alpha",
    proxyDisplayName: "Storybook Pool Attempt Lifecycle",
    endpoint: "/v1/responses",
    model: "gpt-5.4",
    status: "running",
    poolAttemptCount: 1,
    poolDistinctAccountCount: 1,
    inputTokens: 2048,
    cacheInputTokens: 1536,
    totalTokens: 2048,
    requestedServiceTier: "priority",
  };
}

function buildPoolAttemptLifecycleAttempts(
  phase: "connecting" | "sending_request" | "waiting_first_byte" | "streaming_response" | "success",
  occurredAt: string,
): ApiPoolUpstreamRequestAttempt[] {
  if (phase === "success") {
    return [
      {
        attemptId: formatStoryAttemptId(4101),
        invokeId: "inv_storybook_pool_attempt_detail_lifecycle",
        occurredAt,
        endpoint: "/v1/responses",
        stickyKey: "story-pool-attempt-lifecycle",
        upstreamAccountId: 21,
        upstreamAccountName: "Codex Team Alpha",
        upstreamRouteKey: "route-primary",
        proxyBindingKeySnapshot: "fpb_story_pool_attempt_lifecycle",
        attemptIndex: 1,
        distinctAccountIndex: 1,
        sameAccountRetryIndex: 1,
        requesterIp: "203.0.113.42",
        startedAt: occurredAt,
        finishedAt: new Date(Date.parse(occurredAt) + 900).toISOString(),
        status: "success",
        phase: "completed",
        httpStatus: 200,
        connectLatencyMs: 26.4,
        firstByteLatencyMs: 148.2,
        streamLatencyMs: 612.8,
        upstreamRequestId: "req_storybook_pool_attempt_final",
        createdAt: occurredAt,
      },
    ];
  }

  return [
    {
      attemptId: formatStoryAttemptId(4101),
      invokeId: "inv_storybook_pool_attempt_detail_lifecycle",
      occurredAt,
      endpoint: "/v1/responses",
      stickyKey: "story-pool-attempt-lifecycle",
      upstreamAccountId: 21,
      upstreamAccountName: "Codex Team Alpha",
      upstreamRouteKey: "route-primary",
      proxyBindingKeySnapshot: "fpb_story_pool_attempt_lifecycle",
      attemptIndex: 1,
      distinctAccountIndex: 1,
      sameAccountRetryIndex: 1,
      requesterIp: "203.0.113.42",
      startedAt: occurredAt,
      finishedAt: null,
      status: "pending",
      phase,
      httpStatus: null,
      connectLatencyMs:
        phase === "waiting_first_byte" || phase === "streaming_response" ? 26.4 : null,
      firstByteLatencyMs: phase === "streaming_response" ? 148.2 : null,
      streamLatencyMs: null,
      upstreamRequestId: null,
      createdAt: occurredAt,
    },
  ];
}

function PoolAttemptDetailLifecyclePreview() {
  const occurredAtRef = useRef<string>(new Date(Date.now() - 1200).toISOString());
  const [phase, setPhase] = useState<
    "connecting" | "sending_request" | "waiting_first_byte" | "streaming_response" | "success"
  >("connecting");

  useEffect(() => {
    const invokeId = "inv_storybook_pool_attempt_detail_lifecycle";
    storybookPoolAttemptResponses.set(invokeId, () =>
      buildPoolAttemptLifecycleAttempts(phase, occurredAtRef.current),
    );
    return () => {
      storybookPoolAttemptResponses.delete(invokeId);
    };
  }, [phase]);

  useEffect(() => {
    const timers = [
      window.setTimeout(() => setPhase("sending_request"), 400),
      window.setTimeout(() => setPhase("waiting_first_byte"), 950),
      window.setTimeout(() => setPhase("streaming_response"), 1500),
      window.setTimeout(() => setPhase("success"), 2300),
    ];
    return () => {
      timers.forEach((timer) => {
        window.clearTimeout(timer);
      });
    };
  }, []);

  return (
    <InvocationTable
      records={[
        buildPoolAttemptLifecycleRecord(
          phase === "success" ? "success" : "running",
          occurredAtRef.current,
        ),
      ]}
      isLoading={false}
      error={null}
    />
  );
}

const STREAM_VISIBLE_LIMIT = 20;
const STREAM_PROXY_NAMES = [
  "Tokyo-Edge-1",
  "Seoul-Edge-2",
  "Frankfurt-Relay-3",
  "Virginia-Relay-4",
  "Singapore-Edge-5",
  "Sydney-Relay-6",
];
const STREAM_MODELS = ["gpt-5.4", "gpt-5", "gpt-5-mini", "gpt-5.4-mini"];
const STREAM_ENDPOINTS = ["/v1/responses", "/v1/responses/compact", "/v1/chat/completions"];
const STREAM_COMPRESSIONS = ["gzip", "br", "gzip, br"];
const STREAM_REQUEST_TIERS = ["priority", "auto", "flex"] as const;
const STREAM_SUCCESS_TOTAL_MS = [2480, 3920, 5180, 2840, 4630, 3360];
const STREAM_FAILURE_TOTAL_MS = [6120, 8450, 7310, 9280];
const STREAM_TTFB_MS = [118, 166, 241, 384, 92, 211];
const STREAM_MIN_SPAWN_DELAY_MS = 3_000;
const STREAM_MAX_SPAWN_DELAY_MS = 10_000;
const storybookPoolAttemptResponses = new Map<string, () => ApiPoolUpstreamRequestAttempt[]>();

function randomStreamingSpawnDelayMs() {
  return Math.round(
    STREAM_MIN_SPAWN_DELAY_MS +
      Math.random() * (STREAM_MAX_SPAWN_DELAY_MS - STREAM_MIN_SPAWN_DELAY_MS),
  );
}

function defaultStreamingTerminalDurationMs(seq: number, phase: "success" | "failed") {
  if (phase === "failed") {
    return STREAM_FAILURE_TOTAL_MS[seq % STREAM_FAILURE_TOTAL_MS.length] + seq * 41;
  }
  return STREAM_SUCCESS_TOTAL_MS[seq % STREAM_SUCCESS_TOTAL_MS.length] + seq * 27;
}

function clampVisibleRecords(records: ApiInvocation[]): ApiInvocation[] {
  return records
    .slice()
    .sort((left, right) => Date.parse(right.occurredAt) - Date.parse(left.occurredAt))
    .slice(0, STREAM_VISIBLE_LIMIT);
}

function upsertVisibleRecord(records: ApiInvocation[], nextRecord: ApiInvocation): ApiInvocation[] {
  const nextKey = invocationStableKey(nextRecord);
  const index = records.findIndex((record) => invocationStableKey(record) === nextKey);
  if (index === -1) {
    return clampVisibleRecords([nextRecord, ...records]);
  }
  const updated = records.slice();
  updated[index] = nextRecord;
  return clampVisibleRecords(updated);
}

function buildStreamingInvocation(
  seq: number,
  occurredAt: string,
  phase: "initial" | "enriched" | "success" | "failed",
  terminalDurationMs?: number,
): ApiInvocation {
  const routeMode = seq % 3 === 0 ? "forward_proxy" : "pool";
  const upstreamAccountId = routeMode === "pool" ? 21 + (seq % 2) : null;
  const upstreamAccountName =
    routeMode === "pool" ? (seq % 2 === 0 ? "Codex Team Alpha" : "Codex Team Beta") : undefined;
  const requestedServiceTier = STREAM_REQUEST_TIERS[seq % STREAM_REQUEST_TIERS.length];
  const totalMs =
    terminalDurationMs ??
    defaultStreamingTerminalDurationMs(seq, phase === "failed" ? "failed" : "success");
  const ttfbMs = STREAM_TTFB_MS[seq % STREAM_TTFB_MS.length];
  const inputTokens = 1400 + seq * 37;
  const cacheInputTokens = 720 + (seq % 5) * 128;
  const outputTokens = 96 + (seq % 7) * 23;
  const reasoningTokens = 18 + (seq % 4) * 21;
  const stableFields = {
    invokeId: `inv_storybook_stream_${seq}`,
    occurredAt,
    createdAt: occurredAt,
    source: "proxy",
    routeMode,
    upstreamAccountId,
    upstreamAccountName,
    proxyDisplayName: STREAM_PROXY_NAMES[seq % STREAM_PROXY_NAMES.length],
    endpoint: STREAM_ENDPOINTS[seq % STREAM_ENDPOINTS.length],
    model: STREAM_MODELS[seq % STREAM_MODELS.length],
    requestedServiceTier,
  } satisfies Partial<ApiInvocation>;

  if (phase === "initial") {
    return {
      id: -10_000 - seq,
      ...stableFields,
      status: "running",
      inputTokens,
      cacheInputTokens,
      totalTokens: inputTokens,
    } as ApiInvocation;
  }

  if (phase === "enriched") {
    return {
      id: -10_000 - seq,
      ...stableFields,
      status: "running",
      inputTokens,
      cacheInputTokens,
      totalTokens: inputTokens,
      responseContentEncoding: STREAM_COMPRESSIONS[seq % STREAM_COMPRESSIONS.length],
      tUpstreamTtfbMs: ttfbMs,
    } as ApiInvocation;
  }

  if (phase === "failed") {
    return {
      id: 20_000 + seq,
      ...stableFields,
      status: "failed",
      inputTokens,
      cacheInputTokens,
      totalTokens: inputTokens,
      responseContentEncoding: STREAM_COMPRESSIONS[seq % STREAM_COMPRESSIONS.length],
      errorMessage: "upstream timeout while waiting first byte",
      failureKind: "upstream_timeout",
      serviceTier: requestedServiceTier === "priority" ? "auto" : requestedServiceTier,
      proxyWeightDelta: -0.18 - (seq % 4) * 0.11,
      tUpstreamTtfbMs: null,
      tTotalMs: totalMs,
    } as ApiInvocation;
  }

  return {
    id: 20_000 + seq,
    ...stableFields,
    status: "success",
    inputTokens,
    outputTokens,
    cacheInputTokens,
    reasoningTokens,
    reasoningEffort: seq % 3 === 0 ? "medium" : seq % 3 === 1 ? "high" : "low",
    totalTokens: inputTokens + outputTokens,
    cost: Number((0.0028 + seq * 0.00013).toFixed(4)),
    responseContentEncoding: STREAM_COMPRESSIONS[seq % STREAM_COMPRESSIONS.length],
    serviceTier: requestedServiceTier === "flex" ? "flex" : "priority",
    proxyWeightDelta: seq % 5 === 0 ? 0 : Number((0.09 + (seq % 4) * 0.11).toFixed(2)),
    tUpstreamTtfbMs: ttfbMs,
    firstTokenMs: Math.min(totalMs, ttfbMs + 460 + (seq % 4) * 85),
    tTotalMs: totalMs,
  } as ApiInvocation;
}

function buildInitialStreamingRecords(): ApiInvocation[] {
  const now = Date.now();
  const records = Array.from({ length: 16 }, (_, index) => {
    const seq = 1_000 + index;
    const terminalPhase = seq % 6 === 0 ? "failed" : "success";
    const terminalDurationMs = defaultStreamingTerminalDurationMs(seq, terminalPhase);
    const completedAgoMs = (15 - index) * 1800 + 900;
    const occurredAt = new Date(now - completedAgoMs - terminalDurationMs).toISOString();
    return buildStreamingInvocation(seq, occurredAt, terminalPhase, terminalDurationMs);
  });
  return clampVisibleRecords(records);
}

function Recent20StreamingPreview() {
  const [records, setRecords] = useState<ApiInvocation[]>(() => buildInitialStreamingRecords());
  const nextSequenceRef = useRef(2_000);
  const timeoutIdsRef = useRef<number[]>([]);

  useEffect(() => {
    const spawnRecord = () => {
      const seq = nextSequenceRef.current;
      nextSequenceRef.current += 1;
      const occurredAt = new Date().toISOString();
      const enrichDelayMs = 700 + (seq % 3) * 350;
      const terminalDelayMs = 5_000 + (seq % 6) * 1_850;
      const terminalPhase = seq % 5 === 0 ? "failed" : "success";

      setRecords((current) =>
        upsertVisibleRecord(current, buildStreamingInvocation(seq, occurredAt, "initial")),
      );

      timeoutIdsRef.current.push(
        window.setTimeout(() => {
          setRecords((current) => {
            const hasVisibleRecord = current.some(
              (record) =>
                invocationStableKey(record) === `inv_storybook_stream_${seq}-${occurredAt}`,
            );
            if (!hasVisibleRecord) return current;
            return upsertVisibleRecord(
              current,
              buildStreamingInvocation(seq, occurredAt, "enriched"),
            );
          });
        }, enrichDelayMs),
      );

      timeoutIdsRef.current.push(
        window.setTimeout(() => {
          setRecords((current) => {
            const hasVisibleRecord = current.some(
              (record) =>
                invocationStableKey(record) === `inv_storybook_stream_${seq}-${occurredAt}`,
            );
            if (!hasVisibleRecord) return current;
            const elapsedMs = Math.max(0, Date.now() - Date.parse(occurredAt));
            return upsertVisibleRecord(
              current,
              buildStreamingInvocation(
                seq,
                occurredAt,
                terminalPhase,
                Number(elapsedMs.toFixed(1)),
              ),
            );
          });
        }, terminalDelayMs),
      );
    };

    const scheduleNextSpawn = () => {
      const timeoutId = window.setTimeout(() => {
        spawnRecord();
        scheduleNextSpawn();
      }, randomStreamingSpawnDelayMs());
      timeoutIdsRef.current.push(timeoutId);
    };

    spawnRecord();
    scheduleNextSpawn();

    return () => {
      timeoutIdsRef.current.forEach((timeoutId) => {
        window.clearTimeout(timeoutId);
      });
      timeoutIdsRef.current = [];
    };
  }, []);

  return <InvocationTable records={records} isLoading={false} error={null} />;
}

export {
  accountDetails,
  baseOccurredAt,
  buildAccountSummary,
  buildInitialStreamingRecords,
  buildPoolAttemptLifecycleAttempts,
  buildPoolAttemptLifecycleRecord,
  buildStickyConversations,
  buildStreamingInvocation,
  clampVisibleRecords,
  compactLatencyFormattingRecords,
  defaultStreamingTerminalDurationMs,
  endpointBadgeRecords,
  fastIndicatorRecords,
  InvocationTableSharedDrawerPreview,
  InvocationTableStoryShell,
  inFlightResponseDurationUnavailableRecords,
  jsonResponse,
  LONG_PROXY_NAME,
  legacyModelOnlyRecords,
  missingWindowDrawerRecords,
  modelRoutingMismatchRecords,
  PoolAttemptDetailLifecyclePreview,
  poolRoutingAccountStateRecords,
  Recent20StreamingPreview,
  RunningInvocationLifecyclePreview,
  randomStreamingSpawnDelayMs,
  reasoningEffortRecords,
  records,
  requireFixture,
  STREAM_COMPRESSIONS,
  STREAM_ENDPOINTS,
  STREAM_FAILURE_TOTAL_MS,
  STREAM_MAX_SPAWN_DELAY_MS,
  STREAM_MIN_SPAWN_DELAY_MS,
  STREAM_MODELS,
  STREAM_PROXY_NAMES,
  STREAM_REQUEST_TIERS,
  STREAM_SUCCESS_TOTAL_MS,
  STREAM_TTFB_MS,
  STREAM_VISIBLE_LIMIT,
  StorybookInvocationTableMock,
  storybookPoolAttemptResponses,
  upsertVisibleRecord,
};
