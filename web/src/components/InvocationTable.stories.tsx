import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { useEffect, useRef, useState, type ReactNode } from 'react'
import { MemoryRouter, Route, Routes } from 'react-router-dom'
import { I18nProvider } from '../i18n'
import { InvocationTable } from './InvocationTable'
import type { ApiInvocation, UpstreamAccountDetail, UpstreamAccountSummary } from '../lib/api'
import { invocationStableKey } from '../lib/invocation'
import AccountPoolLayout from '../pages/account-pool/AccountPoolLayout'
import UpstreamAccountsPage from '../pages/account-pool/UpstreamAccounts'
import { SystemNotificationProvider } from './ui/system-notifications'

const baseOccurredAt = '2026-02-25T10:15:30Z'
const LONG_PROXY_NAME = 'ivan-hkl-vless-vision-01KFXRNYWYXKN4JHCF3CCV78GD'
const POOL_PROXY_NODE_NAME = 'Ivan-hkl-vless-vision-01KFXRNYWYXKN4JHCF3CCV78GD'
const FORWARD_PROXY_NODE_NAME = 'Ivan-iij-vless-vision-01KHTAANPS3QM1DB4H8FEWMYEW'
const DIRECT_PROXY_NODE_NAME = 'Direct'

const records: ApiInvocation[] = [
  {
    id: 1001,
    invokeId: 'inv_01JSX0PQ3Z8CFQ7AJK8XEH2N4D',
    occurredAt: baseOccurredAt,
    createdAt: baseOccurredAt,
    source: 'proxy',
    routeMode: 'pool',
    upstreamAccountId: 21,
    upstreamAccountName: 'Codex Team Alpha',
    proxyDisplayName: 'Tokyo-Edge-1',
    responseContentEncoding: 'gzip, br',
    endpoint: '/v1/responses',
    model: 'gpt-5-mini',
    status: 'success',
    inputTokens: 1632,
    outputTokens: 298,
    cacheInputTokens: 1240,
    reasoningTokens: 84,
    reasoningEffort: 'high',
    totalTokens: 1930,
    cost: 0.0037,
    requesterIp: '203.0.113.42',
    promptCacheKey: 'pck_6f35b9b20f0348af',
    requestedServiceTier: 'priority',
    serviceTier: 'priority',
    proxyWeightDelta: 0.55,
    tReqReadMs: 1.8,
    tReqParseMs: 3.2,
    tUpstreamConnectMs: 26.1,
    tUpstreamTtfbMs: 184.7,
    tUpstreamStreamMs: 641.9,
    tRespParseMs: 8.6,
    tPersistMs: 2.1,
    tTotalMs: 870.4,
    priceVersion: '2026-02',
  },
  {
    id: 1002,
    invokeId: 'inv_01JSX0Q6YHBFTDVMC3N5NF13R7',
    occurredAt: '2026-02-25T10:18:11Z',
    createdAt: '2026-02-25T10:18:11Z',
    source: 'proxy',
    routeMode: 'forward_proxy',
    proxyDisplayName: LONG_PROXY_NAME,
    responseContentEncoding: 'identity',
    endpoint: '/v1/chat/completions',
    model: 'gpt-5',
    status: 'failed',
    inputTokens: 884,
    outputTokens: 0,
    cacheInputTokens: 0,
    reasoningEffort: 'medium',
    totalTokens: 884,
    errorMessage: 'upstream timeout while waiting first byte',
    failureKind: 'upstream_timeout',
    requestedServiceTier: 'priority',
    serviceTier: 'auto',
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
    invokeId: 'inv_01JSX0R9N0F2V8G54T5PG17WQH',
    occurredAt: '2026-02-25T10:22:48Z',
    createdAt: '2026-02-25T10:22:48Z',
    source: 'proxy',
    routeMode: 'pool',
    upstreamAccountId: 22,
    upstreamAccountName: 'Codex Team Beta',
    proxyDisplayName: 'Seoul-Edge-2',
    responseContentEncoding: 'br',
    endpoint: '/v1/responses',
    model: 'gpt-5.4',
    status: 'success',
    inputTokens: 1520,
    outputTokens: 212,
    cacheInputTokens: 740,
    totalTokens: 1732,
    cost: 0.0051,
    requesterIp: '203.0.113.77',
    promptCacheKey: 'pck_82c89c811a',
    requestedServiceTier: 'priority',
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
]

const accountProxySemanticsRecords: ApiInvocation[] = [
  {
    id: 3001,
    invokeId: 'inv_semantics_pool_named_with_proxy',
    occurredAt: '2026-02-25T12:00:00Z',
    createdAt: '2026-02-25T12:00:00Z',
    source: 'proxy',
    routeMode: 'pool',
    upstreamAccountId: 2,
    upstreamAccountName: 'NSNGC',
    proxyDisplayName: POOL_PROXY_NODE_NAME,
    responseContentEncoding: 'gzip',
    endpoint: '/v1/responses',
    model: 'gpt-5.4',
    status: 'success',
    inputTokens: 4096,
    outputTokens: 96,
    cacheInputTokens: 3584,
    totalTokens: 4192,
    cost: 0.0128,
    tUpstreamTtfbMs: 118.4,
    tTotalMs: 842.7,
  },
  {
    id: 3002,
    invokeId: 'inv_semantics_pool_named_without_proxy',
    occurredAt: '2026-02-25T12:02:00Z',
    createdAt: '2026-02-25T12:02:00Z',
    source: 'proxy',
    routeMode: 'pool',
    upstreamAccountId: 2,
    upstreamAccountName: 'NSNGC',
    responseContentEncoding: 'gzip',
    endpoint: '/v1/responses',
    model: 'gpt-5.4',
    status: 'success',
    inputTokens: 3980,
    outputTokens: 88,
    cacheInputTokens: 3328,
    totalTokens: 4068,
    cost: 0.0119,
    tUpstreamTtfbMs: 120.1,
    tTotalMs: 901.3,
  },
  {
    id: 3003,
    invokeId: 'inv_semantics_pool_id_only_without_proxy',
    occurredAt: '2026-02-25T12:04:00Z',
    createdAt: '2026-02-25T12:04:00Z',
    source: 'proxy',
    routeMode: 'pool',
    upstreamAccountId: 9,
    responseContentEncoding: 'identity',
    endpoint: '/v1/responses',
    model: 'gpt-5-mini',
    status: 'success',
    inputTokens: 1880,
    outputTokens: 64,
    cacheInputTokens: 1440,
    totalTokens: 1944,
    cost: 0.0041,
    tUpstreamTtfbMs: 86.2,
    tTotalMs: 510.8,
  },
  {
    id: 3004,
    invokeId: 'inv_semantics_forward_proxy_with_node',
    occurredAt: '2026-02-25T12:06:00Z',
    createdAt: '2026-02-25T12:06:00Z',
    source: 'proxy',
    routeMode: 'forward_proxy',
    proxyDisplayName: FORWARD_PROXY_NODE_NAME,
    responseContentEncoding: 'br',
    endpoint: '/v1/chat/completions',
    model: 'gpt-5',
    status: 'success',
    inputTokens: 1280,
    outputTokens: 32,
    totalTokens: 1312,
    cost: 0.0036,
    tUpstreamTtfbMs: 144.9,
    tTotalMs: 980.2,
  },
  {
    id: 3005,
    invokeId: 'inv_semantics_forward_proxy_without_node',
    occurredAt: '2026-02-25T12:08:00Z',
    createdAt: '2026-02-25T12:08:00Z',
    source: 'proxy',
    routeMode: 'forward_proxy',
    responseContentEncoding: 'identity',
    endpoint: '/v1/chat/completions',
    model: 'gpt-5',
    status: 'failed',
    inputTokens: 960,
    outputTokens: 0,
    totalTokens: 960,
    errorMessage: 'selected forward proxy missing display name',
    failureKind: 'upstream_timeout',
    tUpstreamTtfbMs: null,
    tTotalMs: 30000.0,
  },
  {
    id: 3006,
    invokeId: 'inv_semantics_legacy_missing_route_mode',
    occurredAt: '2026-02-25T12:10:00Z',
    createdAt: '2026-02-25T12:10:00Z',
    source: 'proxy',
    proxyDisplayName: DIRECT_PROXY_NODE_NAME,
    endpoint: '/v1/responses',
    model: 'gpt-5.4',
    status: 'success',
    inputTokens: 2100,
    outputTokens: 44,
    totalTokens: 2144,
    cost: 0.0052,
    tUpstreamTtfbMs: 72.4,
    tTotalMs: 440.0,
  },
]


const fastIndicatorRecords: ApiInvocation[] = [
  {
    id: 1101,
    invokeId: 'inv_fast_effective',
    occurredAt: '2026-02-25T10:30:00Z',
    createdAt: '2026-02-25T10:30:00Z',
    source: 'proxy',
    proxyDisplayName: 'Fast-effective',
    endpoint: '/v1/responses',
    model: 'gpt-5-mini',
    status: 'success',
    requestedServiceTier: 'priority',
    serviceTier: 'priority',
    inputTokens: 1200,
    outputTokens: 240,
    totalTokens: 1440,
    cost: 0.0032,
    tUpstreamTtfbMs: 118.3,
    tTotalMs: 640.2,
  },
  {
    id: 1102,
    invokeId: 'inv_fast_requested_auto',
    occurredAt: '2026-02-25T10:31:00Z',
    createdAt: '2026-02-25T10:31:00Z',
    source: 'proxy',
    proxyDisplayName: 'Fast-requested-auto',
    endpoint: '/v1/responses',
    model: 'gpt-5',
    status: 'failed',
    requestedServiceTier: 'priority',
    serviceTier: 'auto',
    inputTokens: 980,
    outputTokens: 0,
    totalTokens: 980,
    errorMessage: 'upstream timeout while waiting first byte',
    tUpstreamTtfbMs: null,
    tTotalMs: 30010.5,
  },
  {
    id: 1103,
    invokeId: 'inv_fast_requested_missing',
    occurredAt: '2026-02-25T10:32:00Z',
    createdAt: '2026-02-25T10:32:00Z',
    source: 'proxy',
    proxyDisplayName: 'Fast-requested-missing',
    endpoint: '/v1/responses',
    model: 'gpt-5.4',
    status: 'success',
    requestedServiceTier: 'priority',
    inputTokens: 1024,
    outputTokens: 196,
    totalTokens: 1220,
    cost: 0.0038,
    tUpstreamTtfbMs: 142.6,
    tTotalMs: 702.1,
  },
  {
    id: 1104,
    invokeId: 'inv_fast_effective_auto_request',
    occurredAt: '2026-02-25T10:33:00Z',
    createdAt: '2026-02-25T10:33:00Z',
    source: 'proxy',
    proxyDisplayName: 'Fast-effective-auto-request',
    endpoint: '/v1/responses',
    model: 'gpt-5.4',
    status: 'success',
    requestedServiceTier: 'auto',
    serviceTier: 'priority',
    inputTokens: 1188,
    outputTokens: 202,
    totalTokens: 1390,
    cost: 0.0041,
    tUpstreamTtfbMs: 104.4,
    tTotalMs: 611.9,
  },
  {
    id: 1105,
    invokeId: 'inv_fast_none_flex',
    occurredAt: '2026-02-25T10:34:00Z',
    createdAt: '2026-02-25T10:34:00Z',
    source: 'proxy',
    proxyDisplayName: 'Fast-none-flex',
    endpoint: '/v1/responses',
    model: 'gpt-5.4',
    status: 'success',
    requestedServiceTier: 'flex',
    serviceTier: 'flex',
    inputTokens: 1160,
    outputTokens: 188,
    totalTokens: 1348,
    cost: 0.0035,
    tUpstreamTtfbMs: 156.8,
    tTotalMs: 734.7,
  },
]

const reasoningEffortRecords: ApiInvocation[] = [
  {
    id: 2001,
    invokeId: 'inv_reasoning_none',
    occurredAt: '2026-02-25T11:00:00Z',
    createdAt: '2026-02-25T11:00:00Z',
    source: 'proxy',
    proxyDisplayName: 'Reasoning-none',
    endpoint: '/v1/chat/completions',
    model: 'gpt-5.1',
    status: 'success',
    inputTokens: 640,
    outputTokens: 112,
    cacheInputTokens: 0,
    reasoningEffort: 'none',
    reasoningTokens: 0,
    totalTokens: 752,
    cost: 0.0018,
    tUpstreamTtfbMs: 96.4,
    tTotalMs: 411.7,
  },
  {
    id: 2002,
    invokeId: 'inv_reasoning_minimal',
    occurredAt: '2026-02-25T11:02:00Z',
    createdAt: '2026-02-25T11:02:00Z',
    source: 'proxy',
    proxyDisplayName: 'Reasoning-minimal',
    endpoint: '/v1/responses',
    model: 'gpt-5',
    status: 'success',
    inputTokens: 712,
    outputTokens: 144,
    cacheInputTokens: 128,
    reasoningEffort: 'minimal',
    reasoningTokens: 12,
    totalTokens: 856,
    cost: 0.0021,
    tUpstreamTtfbMs: 118.1,
    tTotalMs: 588.2,
  },
  {
    id: 2003,
    invokeId: 'inv_reasoning_low',
    occurredAt: '2026-02-25T11:04:00Z',
    createdAt: '2026-02-25T11:04:00Z',
    source: 'proxy',
    proxyDisplayName: 'Reasoning-low',
    endpoint: '/v1/responses',
    model: 'gpt-5-mini',
    status: 'success',
    inputTokens: 804,
    outputTokens: 166,
    cacheInputTokens: 256,
    reasoningEffort: 'low',
    reasoningTokens: 28,
    totalTokens: 970,
    cost: 0.0024,
    tUpstreamTtfbMs: 132.5,
    tTotalMs: 710.4,
  },
  {
    id: 2004,
    invokeId: 'inv_reasoning_medium',
    occurredAt: '2026-02-25T11:06:00Z',
    createdAt: '2026-02-25T11:06:00Z',
    source: 'proxy',
    proxyDisplayName: 'Reasoning-medium',
    endpoint: '/v1/chat/completions',
    model: 'gpt-5',
    status: 'failed',
    inputTokens: 920,
    outputTokens: 0,
    cacheInputTokens: 0,
    reasoningEffort: 'medium',
    totalTokens: 920,
    errorMessage: 'upstream timeout while waiting first byte',
    failureKind: 'upstream_timeout',
    tUpstreamTtfbMs: null,
    tTotalMs: 30012.0,
  },
  {
    id: 2005,
    invokeId: 'inv_reasoning_high',
    occurredAt: '2026-02-25T11:08:00Z',
    createdAt: '2026-02-25T11:08:00Z',
    source: 'proxy',
    proxyDisplayName: 'Reasoning-high',
    endpoint: '/v1/responses',
    model: 'gpt-5',
    status: 'success',
    inputTokens: 1012,
    outputTokens: 244,
    cacheInputTokens: 320,
    reasoningEffort: 'high',
    reasoningTokens: 84,
    totalTokens: 1256,
    cost: 0.0031,
    tUpstreamTtfbMs: 188.4,
    tTotalMs: 962.6,
  },
  {
    id: 2006,
    invokeId: 'inv_reasoning_xhigh',
    occurredAt: '2026-02-25T11:10:00Z',
    createdAt: '2026-02-25T11:10:00Z',
    source: 'proxy',
    proxyDisplayName: 'Reasoning-xhigh',
    endpoint: '/v1/responses',
    model: 'gpt-5.2',
    status: 'success',
    inputTokens: 1130,
    outputTokens: 318,
    cacheInputTokens: 512,
    reasoningEffort: 'xhigh',
    reasoningTokens: 146,
    totalTokens: 1448,
    cost: 0.0048,
    tUpstreamTtfbMs: 261.3,
    tTotalMs: 1384.9,
  },
  {
    id: 2007,
    invokeId: 'inv_reasoning_missing',
    occurredAt: '2026-02-25T11:12:00Z',
    createdAt: '2026-02-25T11:12:00Z',
    source: 'proxy',
    proxyDisplayName: 'Reasoning-missing',
    endpoint: '/v1/responses',
    model: 'gpt-5-mini',
    status: 'success',
    inputTokens: 540,
    outputTokens: 90,
    cacheInputTokens: 64,
    totalTokens: 630,
    cost: 0.0015,
    tUpstreamTtfbMs: 104.7,
    tTotalMs: 498.5,
  },
  {
    id: 2008,
    invokeId: 'inv_reasoning_unknown',
    occurredAt: '2026-02-25T11:14:00Z',
    createdAt: '2026-02-25T11:14:00Z',
    source: 'proxy',
    proxyDisplayName: 'Reasoning-unknown',
    endpoint: '/v1/responses',
    model: 'custom-reasoning-model',
    status: 'success',
    inputTokens: 600,
    outputTokens: 120,
    cacheInputTokens: 0,
    reasoningEffort: 'custom-tier',
    reasoningTokens: 33,
    totalTokens: 720,
    cost: 0.0019,
    tUpstreamTtfbMs: 124.2,
    tTotalMs: 544.0,
  },
]

const accountDetails = new Map<number, UpstreamAccountDetail>([
  [
    2,
    {
      id: 2,
      kind: 'oauth_codex',
      provider: 'openai',
      displayName: 'NSNGC',
      groupName: 'nsngc',
      isMother: false,
      status: 'active',
      enabled: true,
      email: 'nsngc@example.com',
      chatgptAccountId: 'org_nsngc',
      chatgptUserId: 'user_nsngc',
      planType: 'team',
      maskedApiKey: null,
      lastSyncedAt: '2026-03-16T09:12:00Z',
      lastSuccessfulSyncAt: '2026-03-16T09:11:00Z',
      lastError: null,
      lastErrorAt: null,
      tokenExpiresAt: '2026-03-16T12:15:00Z',
      lastRefreshedAt: '2026-03-16T09:11:30Z',
      primaryWindow: {
        usedPercent: 18,
        usedText: '18 / 100',
        limitText: '100 requests',
        resetsAt: '2026-03-16T10:00:00Z',
        windowDurationMins: 300,
      },
      secondaryWindow: {
        usedPercent: 31,
        usedText: '31 / 100',
        limitText: '100 requests',
        resetsAt: '2026-03-17T00:00:00Z',
        windowDurationMins: 10080,
      },
      credits: null,
      localLimits: null,
      duplicateInfo: null,
      tags: [],
      effectiveRoutingRule: {
        guardEnabled: false,
        lookbackHours: null,
        maxConversations: null,
        allowCutOut: true,
        allowCutIn: true,
        sourceTagIds: [],
        sourceTagNames: [],
        guardRules: [],
      },
      note: null,
      upstreamBaseUrl: 'https://claude-relay-service.nsngc.org',
      history: [],
    },
  ],
  [
    9,
    {
      id: 9,
      kind: 'oauth_codex',
      provider: 'openai',
      displayName: 'Fallback Account 9',
      groupName: 'fallback',
      isMother: false,
      status: 'active',
      enabled: true,
      email: 'fallback9@example.com',
      chatgptAccountId: 'org_fallback_9',
      chatgptUserId: 'user_fallback_9',
      planType: 'pro',
      maskedApiKey: null,
      lastSyncedAt: '2026-03-16T07:12:00Z',
      lastSuccessfulSyncAt: '2026-03-16T07:11:00Z',
      lastError: null,
      lastErrorAt: null,
      tokenExpiresAt: '2026-03-16T11:45:00Z',
      lastRefreshedAt: '2026-03-16T07:11:30Z',
      primaryWindow: {
        usedPercent: 9,
        usedText: '9 / 100',
        limitText: '100 requests',
        resetsAt: '2026-03-16T10:00:00Z',
        windowDurationMins: 300,
      },
      secondaryWindow: {
        usedPercent: 14,
        usedText: '14 / 100',
        limitText: '100 requests',
        resetsAt: '2026-03-17T00:00:00Z',
        windowDurationMins: 10080,
      },
      credits: null,
      localLimits: null,
      duplicateInfo: null,
      tags: [],
      effectiveRoutingRule: {
        guardEnabled: false,
        lookbackHours: null,
        maxConversations: null,
        allowCutOut: true,
        allowCutIn: true,
        sourceTagIds: [],
        sourceTagNames: [],
        guardRules: [],
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
      kind: 'oauth_codex',
      provider: 'openai',
      displayName: 'Codex Team Alpha',
      groupName: 'team-alpha',
      isMother: true,
      status: 'active',
      enabled: true,
      email: 'alpha@example.com',
      chatgptAccountId: 'org_alpha',
      chatgptUserId: 'user_alpha',
      planType: 'team',
      maskedApiKey: null,
      lastSyncedAt: '2026-03-16T09:10:00Z',
      lastSuccessfulSyncAt: '2026-03-16T09:08:00Z',
      lastError: null,
      lastErrorAt: null,
      tokenExpiresAt: '2026-03-16T12:00:00Z',
      lastRefreshedAt: '2026-03-16T09:09:00Z',
      primaryWindow: {
        usedPercent: 22,
        usedText: '22 / 100',
        limitText: '100 requests',
        resetsAt: '2026-03-16T10:00:00Z',
        windowDurationMins: 300,
      },
      secondaryWindow: {
        usedPercent: 36,
        usedText: '36 / 100',
        limitText: '100 requests',
        resetsAt: '2026-03-17T00:00:00Z',
        windowDurationMins: 10080,
      },
      credits: null,
      localLimits: null,
      duplicateInfo: null,
      tags: [],
      effectiveRoutingRule: {
        guardEnabled: false,
        lookbackHours: null,
        maxConversations: null,
        allowCutOut: true,
        allowCutIn: true,
        sourceTagIds: [],
        sourceTagNames: [],
        guardRules: [],
      },
      note: null,
      upstreamBaseUrl: null,
      history: [],
    },
  ],
  [
    22,
    {
      id: 22,
      kind: 'oauth_codex',
      provider: 'openai',
      displayName: 'Codex Team Beta',
      groupName: 'team-beta',
      isMother: false,
      status: 'active',
      enabled: true,
      email: 'beta@example.com',
      chatgptAccountId: 'org_beta',
      chatgptUserId: 'user_beta',
      planType: 'pro',
      maskedApiKey: null,
      lastSyncedAt: '2026-03-16T08:20:00Z',
      lastSuccessfulSyncAt: '2026-03-16T08:19:00Z',
      lastError: null,
      lastErrorAt: null,
      tokenExpiresAt: '2026-03-16T11:50:00Z',
      lastRefreshedAt: '2026-03-16T08:19:30Z',
      primaryWindow: {
        usedPercent: 48,
        usedText: '48 / 100',
        limitText: '100 requests',
        resetsAt: '2026-03-16T10:00:00Z',
        windowDurationMins: 300,
      },
      secondaryWindow: {
        usedPercent: 52,
        usedText: '52 / 100',
        limitText: '100 requests',
        resetsAt: '2026-03-17T00:00:00Z',
        windowDurationMins: 10080,
      },
      credits: null,
      localLimits: null,
      duplicateInfo: null,
      tags: [],
      effectiveRoutingRule: {
        guardEnabled: false,
        lookbackHours: null,
        maxConversations: null,
        allowCutOut: true,
        allowCutIn: true,
        sourceTagIds: [],
        sourceTagNames: [],
        guardRules: [],
      },
      note: null,
      upstreamBaseUrl: null,
      history: [],
    },
  ],
])

function jsonResponse(body: unknown, status = 200) {
  return Promise.resolve(
    new Response(JSON.stringify(body), {
      status,
      headers: {
        'Content-Type': 'application/json',
      },
    }),
  )
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
  }
}

function buildStickyConversations(accountId: number) {
  return {
    rangeStart: '2026-03-16T00:00:00Z',
    rangeEnd: '2026-03-17T00:00:00Z',
    conversations:
      accountId === 21
        ? [
            {
              stickyKey: '019ce3a1-6787-7910-b0fd-c246d6f6a901',
              requestCount: 10,
              totalTokens: 455170,
              totalCost: 0.3507,
              createdAt: '2026-03-16T04:01:20.000Z',
              lastActivityAt: '2026-03-16T04:03:02.000Z',
              last24hRequests: [
                {
                  occurredAt: '2026-03-16T10:15:00.000Z',
                  status: 'success',
                  isSuccess: true,
                  requestTokens: 102440,
                  cumulativeTokens: 102440,
                },
                {
                  occurredAt: '2026-03-16T18:20:00.000Z',
                  status: 'success',
                  isSuccess: true,
                  requestTokens: 154380,
                  cumulativeTokens: 256820,
                },
              ],
            },
          ]
        : [],
  }
}

function StorybookInvocationTableMock({ children }: { children: ReactNode }) {
  const originalFetchRef = useRef<typeof window.fetch | null>(null)

  if (typeof window !== 'undefined' && originalFetchRef.current == null) {
    originalFetchRef.current = window.fetch.bind(window)
    window.fetch = async (input, init) => {
      const request = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url
      const method =
        init?.method ??
        (typeof input === 'string' || input instanceof URL ? 'GET' : input.method)

      if (method.toUpperCase() === 'GET') {
        const url = new URL(request, window.location.origin)
        if (url.pathname === '/api/pool/upstream-accounts') {
          const items = Array.from(accountDetails.values()).map(buildAccountSummary)
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
              maskedApiKey: 'pool-live••••••c0de',
            },
          })
        }
        if (url.pathname === '/api/pool/tags') {
          return jsonResponse({
            writesEnabled: true,
            items: [],
          })
        }
        const match = url.pathname.match(/^\/api\/pool\/upstream-accounts\/(\d+)$/)
        if (match) {
          const detail = accountDetails.get(Number(match[1]))
          if (detail) return jsonResponse(detail)
          return jsonResponse({ message: 'Not found' }, 404)
        }
        const stickyMatch = url.pathname.match(/^\/api\/pool\/upstream-accounts\/(\d+)\/sticky-keys$/)
        if (stickyMatch) {
          return jsonResponse(buildStickyConversations(Number(stickyMatch[1])))
        }
      }

      return originalFetchRef.current
        ? originalFetchRef.current(input as Parameters<typeof fetch>[0], init)
        : fetch(input as Parameters<typeof fetch>[0], init)
    }
  }

  useEffect(() => {
    return () => {
      if (originalFetchRef.current) {
        window.fetch = originalFetchRef.current
      }
    }
  }, [])

  return <>{children}</>
}

function InvocationTableStoryShell({ children }: { children: ReactNode }) {
  return (
    <div className="bg-base-200 px-6 py-6 text-base-content">
      <div className="mx-auto w-full max-w-6xl p-6">
        <section className="card bg-base-100 shadow-sm">
          <div className="card-body gap-4">{children}</div>
        </section>
      </div>
    </div>
  )
}

function RunningInvocationLifecyclePreview() {
  const occurredAtRef = useRef<string>(new Date(Date.now() - 1200).toISOString())
  const [phase, setPhase] = useState<'initial' | 'enriched' | 'terminal'>('initial')

  useEffect(() => {
    const enrichTimer = window.setTimeout(() => setPhase('enriched'), 1200)
    const terminalTimer = window.setTimeout(() => setPhase('terminal'), 2800)
    return () => {
      window.clearTimeout(enrichTimer)
      window.clearTimeout(terminalTimer)
    }
  }, [])

  const occurredAt = occurredAtRef.current
  const terminalElapsedMs = Math.max(0, Date.now() - Date.parse(occurredAt))
  const lifecycleRecord: ApiInvocation =
    phase === 'terminal'
      ? {
          id: 1201,
          invokeId: 'inv_storybook_running_lifecycle',
          occurredAt,
          createdAt: occurredAt,
          source: 'proxy',
          routeMode: 'pool',
          upstreamAccountId: 21,
          upstreamAccountName: 'Codex Team Alpha',
          proxyDisplayName: 'Storybook Live Running Demo',
          responseContentEncoding: 'gzip, br',
          endpoint: '/v1/responses/compact',
          model: 'gpt-5.4',
          status: 'success',
          inputTokens: 2048,
          outputTokens: 188,
          cacheInputTokens: 1536,
          reasoningTokens: 64,
          reasoningEffort: 'high',
          totalTokens: 2236,
          cost: 0.0046,
          requestedServiceTier: 'priority',
          serviceTier: 'priority',
          proxyWeightDelta: 0.42,
          tUpstreamTtfbMs: 184.2,
          tTotalMs: Number(terminalElapsedMs.toFixed(1)),
        }
      : {
          id: -1201,
          invokeId: 'inv_storybook_running_lifecycle',
          occurredAt,
          createdAt: occurredAt,
          source: 'proxy',
          routeMode: 'pool',
          upstreamAccountId: 21,
          upstreamAccountName: 'Codex Team Alpha',
          proxyDisplayName: 'Storybook Live Running Demo',
          endpoint: '/v1/responses/compact',
          model: 'gpt-5.4',
          status: 'running',
          inputTokens: 2048,
          cacheInputTokens: 1536,
          totalTokens: 2048,
          requestedServiceTier: 'priority',
          responseContentEncoding: phase === 'enriched' ? 'gzip' : undefined,
          tUpstreamTtfbMs: phase === 'enriched' ? 184.2 : null,
        }

  return <InvocationTable records={[lifecycleRecord]} isLoading={false} error={null} />
}

const STREAM_VISIBLE_LIMIT = 20
const STREAM_PROXY_NAMES = [
  'Tokyo-Edge-1',
  'Seoul-Edge-2',
  'Frankfurt-Relay-3',
  'Virginia-Relay-4',
  'Singapore-Edge-5',
  'Sydney-Relay-6',
]
const STREAM_MODELS = ['gpt-5.4', 'gpt-5', 'gpt-5-mini', 'gpt-5.4-mini']
const STREAM_ENDPOINTS = ['/v1/responses', '/v1/responses/compact', '/v1/chat/completions']
const STREAM_COMPRESSIONS = ['gzip', 'br', 'gzip, br']
const STREAM_REQUEST_TIERS = ['priority', 'auto', 'flex'] as const
const STREAM_SUCCESS_TOTAL_MS = [2480, 3920, 5180, 2840, 4630, 3360]
const STREAM_FAILURE_TOTAL_MS = [6120, 8450, 7310, 9280]
const STREAM_TTFB_MS = [118, 166, 241, 384, 92, 211]
const STREAM_MIN_SPAWN_DELAY_MS = 3_000
const STREAM_MAX_SPAWN_DELAY_MS = 10_000

function randomStreamingSpawnDelayMs() {
  return Math.round(
    STREAM_MIN_SPAWN_DELAY_MS +
      Math.random() * (STREAM_MAX_SPAWN_DELAY_MS - STREAM_MIN_SPAWN_DELAY_MS),
  )
}

function defaultStreamingTerminalDurationMs(seq: number, phase: 'success' | 'failed') {
  if (phase === 'failed') {
    return STREAM_FAILURE_TOTAL_MS[seq % STREAM_FAILURE_TOTAL_MS.length] + seq * 41
  }
  return STREAM_SUCCESS_TOTAL_MS[seq % STREAM_SUCCESS_TOTAL_MS.length] + seq * 27
}

function clampVisibleRecords(records: ApiInvocation[]): ApiInvocation[] {
  return records
    .slice()
    .sort((left, right) => Date.parse(right.occurredAt) - Date.parse(left.occurredAt))
    .slice(0, STREAM_VISIBLE_LIMIT)
}

function upsertVisibleRecord(records: ApiInvocation[], nextRecord: ApiInvocation): ApiInvocation[] {
  const nextKey = invocationStableKey(nextRecord)
  const index = records.findIndex((record) => invocationStableKey(record) === nextKey)
  if (index === -1) {
    return clampVisibleRecords([nextRecord, ...records])
  }
  const updated = records.slice()
  updated[index] = nextRecord
  return clampVisibleRecords(updated)
}

function buildStreamingInvocation(
  seq: number,
  occurredAt: string,
  phase: 'initial' | 'enriched' | 'success' | 'failed',
  terminalDurationMs?: number,
): ApiInvocation {
  const routeMode = seq % 3 === 0 ? 'forward_proxy' : 'pool'
  const upstreamAccountId = routeMode === 'pool' ? 21 + (seq % 2) : null
  const upstreamAccountName = routeMode === 'pool' ? (seq % 2 === 0 ? 'Codex Team Alpha' : 'Codex Team Beta') : undefined
  const requestedServiceTier = STREAM_REQUEST_TIERS[seq % STREAM_REQUEST_TIERS.length]
  const totalMs =
    terminalDurationMs ??
    defaultStreamingTerminalDurationMs(seq, phase === 'failed' ? 'failed' : 'success')
  const ttfbMs = STREAM_TTFB_MS[seq % STREAM_TTFB_MS.length]
  const inputTokens = 1400 + seq * 37
  const cacheInputTokens = 720 + (seq % 5) * 128
  const outputTokens = 96 + (seq % 7) * 23
  const reasoningTokens = 18 + (seq % 4) * 21
  const stableFields = {
    invokeId: `inv_storybook_stream_${seq}`,
    occurredAt,
    createdAt: occurredAt,
    source: 'proxy',
    routeMode,
    upstreamAccountId,
    upstreamAccountName,
    proxyDisplayName: STREAM_PROXY_NAMES[seq % STREAM_PROXY_NAMES.length],
    endpoint: STREAM_ENDPOINTS[seq % STREAM_ENDPOINTS.length],
    model: STREAM_MODELS[seq % STREAM_MODELS.length],
    requestedServiceTier,
  } satisfies Partial<ApiInvocation>

  if (phase === 'initial') {
    return {
      id: -10_000 - seq,
      ...stableFields,
      status: 'running',
      inputTokens,
      cacheInputTokens,
      totalTokens: inputTokens,
    } as ApiInvocation
  }

  if (phase === 'enriched') {
    return {
      id: -10_000 - seq,
      ...stableFields,
      status: 'running',
      inputTokens,
      cacheInputTokens,
      totalTokens: inputTokens,
      responseContentEncoding: STREAM_COMPRESSIONS[seq % STREAM_COMPRESSIONS.length],
      tUpstreamTtfbMs: ttfbMs,
    } as ApiInvocation
  }

  if (phase === 'failed') {
    return {
      id: 20_000 + seq,
      ...stableFields,
      status: 'failed',
      inputTokens,
      cacheInputTokens,
      totalTokens: inputTokens,
      responseContentEncoding: STREAM_COMPRESSIONS[seq % STREAM_COMPRESSIONS.length],
      errorMessage: 'upstream timeout while waiting first byte',
      failureKind: 'upstream_timeout',
      serviceTier: requestedServiceTier === 'priority' ? 'auto' : requestedServiceTier,
      proxyWeightDelta: -0.18 - (seq % 4) * 0.11,
      tUpstreamTtfbMs: null,
      tTotalMs: totalMs,
    } as ApiInvocation
  }

  return {
    id: 20_000 + seq,
    ...stableFields,
    status: 'success',
    inputTokens,
    outputTokens,
    cacheInputTokens,
    reasoningTokens,
    reasoningEffort: seq % 3 === 0 ? 'medium' : seq % 3 === 1 ? 'high' : 'low',
    totalTokens: inputTokens + outputTokens,
    cost: Number((0.0028 + seq * 0.00013).toFixed(4)),
    responseContentEncoding: STREAM_COMPRESSIONS[seq % STREAM_COMPRESSIONS.length],
    serviceTier: requestedServiceTier === 'flex' ? 'flex' : 'priority',
    proxyWeightDelta: seq % 5 === 0 ? 0 : Number((0.09 + (seq % 4) * 0.11).toFixed(2)),
    tUpstreamTtfbMs: ttfbMs,
    tTotalMs: totalMs,
  } as ApiInvocation
}

function buildInitialStreamingRecords(): ApiInvocation[] {
  const now = Date.now()
  const records = Array.from({ length: 16 }, (_, index) => {
    const seq = 1_000 + index
    const terminalPhase = seq % 6 === 0 ? 'failed' : 'success'
    const terminalDurationMs = defaultStreamingTerminalDurationMs(seq, terminalPhase)
    const completedAgoMs = (15 - index) * 1800 + 900
    const occurredAt = new Date(now - completedAgoMs - terminalDurationMs).toISOString()
    return buildStreamingInvocation(seq, occurredAt, terminalPhase, terminalDurationMs)
  })
  return clampVisibleRecords(records)
}

function Recent20StreamingPreview() {
  const [records, setRecords] = useState<ApiInvocation[]>(() => buildInitialStreamingRecords())
  const nextSequenceRef = useRef(2_000)
  const timeoutIdsRef = useRef<number[]>([])

  useEffect(() => {
    const spawnRecord = () => {
      const seq = nextSequenceRef.current
      nextSequenceRef.current += 1
      const occurredAt = new Date().toISOString()
      const enrichDelayMs = 700 + (seq % 3) * 350
      const terminalDelayMs = 5_000 + (seq % 6) * 1_850
      const terminalPhase = seq % 5 === 0 ? 'failed' : 'success'

      setRecords((current) => upsertVisibleRecord(current, buildStreamingInvocation(seq, occurredAt, 'initial')))

      timeoutIdsRef.current.push(
        window.setTimeout(() => {
          setRecords((current) => {
            const hasVisibleRecord = current.some((record) => invocationStableKey(record) === `inv_storybook_stream_${seq}-${occurredAt}`)
            if (!hasVisibleRecord) return current
            return upsertVisibleRecord(current, buildStreamingInvocation(seq, occurredAt, 'enriched'))
          })
        }, enrichDelayMs),
      )

      timeoutIdsRef.current.push(
        window.setTimeout(() => {
          setRecords((current) => {
            const hasVisibleRecord = current.some((record) => invocationStableKey(record) === `inv_storybook_stream_${seq}-${occurredAt}`)
            if (!hasVisibleRecord) return current
            const elapsedMs = Math.max(0, Date.now() - Date.parse(occurredAt))
            return upsertVisibleRecord(
              current,
              buildStreamingInvocation(seq, occurredAt, terminalPhase, Number(elapsedMs.toFixed(1))),
            )
          })
        }, terminalDelayMs),
      )
    }

    const scheduleNextSpawn = () => {
      const timeoutId = window.setTimeout(() => {
        spawnRecord()
        scheduleNextSpawn()
      }, randomStreamingSpawnDelayMs())
      timeoutIdsRef.current.push(timeoutId)
    }

    spawnRecord()
    scheduleNextSpawn()

    return () => {
      timeoutIdsRef.current.forEach((timeoutId) => window.clearTimeout(timeoutId))
      timeoutIdsRef.current = []
    }
  }, [])

  return <InvocationTable records={records} isLoading={false} error={null} />
}

const meta = {
  title: 'Monitoring/InvocationTable',
  component: InvocationTable,
  tags: ['autodocs'],
  parameters: {
    layout: 'fullscreen',
    docs: {
      description: {
        component:
          'Shows recent invocation records with status, account attribution, proxy metadata, elapsed/compression summaries, and expandable request details. The default story includes both pool-routed and reverse-proxy records so you can verify the `账号 / 代理` split, the dedicated `用时` column, and the current-page account drawer trigger. The output summary still shows output tokens on the first line and the reasoning-token breakdown on the second line.\n\nThe `账号 / 代理` column follows a strict semantic split: the first line identifies who sent the request (`号池账号名` / `账号 #<id>` / `反向代理`), while the second line identifies the true forward-proxy node and may only show a real proxy display name or `—`. Upstream hosts such as `claude-relay-service.nsngc.org`, `chatgpt.com`, or `api.openai.com` are never valid proxy-line values. Use the `Account Proxy Semantics` story to review the supported combinations side by side.\n\nVisible reasoning effort cases in this component: `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, missing (`—`), and unknown raw strings such as `custom-tier`. The component only shows explicitly recorded request values and does not infer model defaults. According to the OpenAI API docs as checked on 2026-03-07, the general API-level values are `none`, `minimal`, `low`, `medium`, `high`, and `xhigh`, but model support is narrower for some models.\n\nReasoning-effort colors now follow a stable ladder: `none` stays neutral, `minimal/low` use cool informational tones, `medium` moves into the primary tier, `high` warns in amber, `xhigh` escalates to error red, and unknown raw strings use a dashed neutral badge so they cannot be mistaken for a standard level.\n\nUse this component to verify the summary row layout on desktop, the card layout on mobile, the account/proxy semantics matrix, the running-to-terminal live update story, and the expanded detail section for request metadata, timing stages, account attribution, and HTTP compression.',
      },
    },
  },
  argTypes: {
    records: {
      control: 'object',
      description:
        'Invocation rows rendered by the table. Include `reasoningEffort` to show the summary badge and `reasoningTokens` to populate both the output-column breakdown and the expanded detail field; missing values render as `—`.',
      table: {
        type: { summary: 'ApiInvocation[]' },
      },
    },
    isLoading: {
      control: 'boolean',
      description: 'Displays the loading spinner state while the table is waiting for records.',
      table: {
        type: { summary: 'boolean' },
        defaultValue: { summary: 'false' },
      },
    },
    error: {
      control: 'text',
      description: 'Optional request error message rendered above the table when loading fails.',
      table: {
        type: { summary: 'string | null' },
        defaultValue: { summary: 'null' },
      },
    },
  },
  decorators: [
    (Story) => (
      <I18nProvider>
        <MemoryRouter initialEntries={['/dashboard']}>
          <SystemNotificationProvider>
            <StorybookInvocationTableMock>
              <Routes>
                <Route
                  path="/dashboard"
                  element={
                    <InvocationTableStoryShell>
                      <Story />
                    </InvocationTableStoryShell>
                  }
                />
                <Route path="/account-pool" element={<AccountPoolLayout />}>
                  <Route path="upstream-accounts" element={<UpstreamAccountsPage />} />
                </Route>
              </Routes>
            </StorybookInvocationTableMock>
          </SystemNotificationProvider>
        </MemoryRouter>
      </I18nProvider>
    ),
  ],
} satisfies Meta<typeof InvocationTable>

export default meta

type Story = StoryObj<typeof meta>

const defaultArgs: Story['args'] = {
  records,
  isLoading: false,
  error: null,
}

export const Default: Story = {
  args: defaultArgs,
  parameters: {
    docs: {
      description: {
        story:
          'Reference state with pool-routed and reverse-proxy invocations. Verify the `账号 / 代理` split, the dedicated elapsed/compression column, and the reasoning-token breakdown in the output summary.',
      },
    },
  },
}

export const RunningLifecycleSimulation: Story = {
  render: () => <RunningInvocationLifecyclePreview />,

  parameters: {
    docs: {
      description: {
        story:
          'Mock story for the new live-running experience: the row appears immediately as `running`, later receives TTFB + HTTP compression context, and finally swaps in the terminal persisted record without duplicating the row. The terminal step intentionally switches from a negative temporary id to a positive persisted id while keeping the same `invokeId + occurredAt` stable key.',
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText(/Storybook Live Running Demo/i)).toBeInTheDocument()
    await expect(canvas.getByText(/运行中|running/i)).toBeInTheDocument()

    const toggleButtons = await canvas.findAllByRole('button', { name: /展开详情|show details/i })
    await userEvent.click(toggleButtons[0])

    await waitFor(
      async () => {
        await expect(canvas.getByText(/HTTP 压缩算法|http compression/i)).toBeInTheDocument()
        await expect(canvas.getByText(/gzip/i)).toBeInTheDocument()
      },
      { timeout: 4000 },
    )

    await waitFor(
      async () => {
        await expect(canvas.getByText(/成功|success/i)).toBeInTheDocument()
      },
      { timeout: 5000 },
    )
  },
}

export const AccountProxySemantics: Story = {
  args: {
    records: accountProxySemanticsRecords,
    isLoading: false,
    error: null,
  },
  parameters: {
    docs: {
      description: {
        story:
          'Curated semantics matrix for the `账号 / 代理` column. It demonstrates the six supported combinations: `pool + 账号名 + 真实代理节点`, `pool + 账号名 + 无代理节点`, `pool + 仅账号 ID`, `forward_proxy + 真实代理节点`, `forward_proxy + 无代理节点`, and `历史记录缺 routeMode 的降级展示`. In every case, the first line is the routing identity and the second line is only the real proxy node or `—`; upstream hosts must never appear here.',
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getAllByRole('button', { name: 'NSNGC' })).toHaveLength(2)
    await expect(canvas.getByRole('button', { name: '账号 #9' })).toBeInTheDocument()
    await expect(canvas.queryByRole('button', { name: '反向代理' })).not.toBeInTheDocument()
    await expect(canvas.getByText(POOL_PROXY_NODE_NAME)).toBeInTheDocument()
    await expect(canvas.getByText(FORWARD_PROXY_NODE_NAME)).toBeInTheDocument()
    await expect(canvas.getByText(DIRECT_PROXY_NODE_NAME)).toBeInTheDocument()
    await expect(canvas.queryByText('claude-relay-service.nsngc.org')).not.toBeInTheDocument()
  },
}

export const Recent20StreamingSimulation: Story = {
  render: () => <Recent20StreamingPreview />,
  parameters: {
    docs: {
      description: {
        story:
          'Simulates the “最近 20 条实况” surface with a continuously moving stream: new requests keep appearing at the top, several rows remain in `running`, and each request finishes after a different delay so the table mixes success, failure, and in-flight elapsed timers at the same time. New arrivals are randomized between 3 and 10 seconds to better match a real monitoring feed, and the canvas stays capped near 20 visible rows to mirror the real dashboard/live summary view.',
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)

    await waitFor(
      async () => {
        const rowCount = canvasElement.querySelectorAll('tbody > tr').length
        expect(rowCount).toBeGreaterThanOrEqual(12)
      },
      { timeout: 3000 },
    )

    await waitFor(
      async () => {
        await expect(canvas.getByText(/运行中|running/i)).toBeInTheDocument()
      },
      { timeout: 5000 },
    )

    await waitFor(
      async () => {
        const statusText = canvasElement.textContent ?? ''
        expect(/成功|success/i.test(statusText)).toBe(true)
        expect(/失败|failed/i.test(statusText)).toBe(true)
      },
      { timeout: 7000 },
    )
  },
}

export const ExpandedDetails: Story = {
  args: defaultArgs,
  parameters: {
    docs: {
      description: {
        story:
          'Auto-expands the first invocation so you can review the default open detail layout, including account attribution, latency fields, and timing-stage breakdown without needing manual interaction.',
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const toggleButtons = await canvas.findAllByRole('button', { name: /展开详情|show details/i })
    await userEvent.click(toggleButtons[0])
    await expect(canvas.getByText(/请求详情|request details/i)).toBeInTheDocument()
    await expect(canvas.getByText(/HTTP 压缩算法|http compression/i)).toBeInTheDocument()
  },
}

export const AccountDrawer: Story = {
  args: defaultArgs,
  parameters: {
    docs: {
      description: {
        story:
          'Clicks the first pool account badge and verifies that the current-page read-only account drawer opens with mocked account detail data.',
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const documentScope = within(canvasElement.ownerDocument.body)
    await userEvent.click(await canvas.findByRole('button', { name: 'Codex Team Alpha' }))
    await expect(documentScope.getByRole('dialog', { name: /Codex Team Alpha/i })).toBeInTheDocument()
    await expect(documentScope.getByText(/去号池查看完整详情|Open in account pool/i)).toBeInTheDocument()
  },
}

export const AccountPoolDestination: Story = {
  args: defaultArgs,
  parameters: {
    docs: {
      description: {
        story:
          'Opens the account drawer and follows the “去号池查看完整详情” action so you can preview the destination account-pool page with the matching account detail already selected.',
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const documentScope = within(canvasElement.ownerDocument.body)
    await userEvent.click(await canvas.findByRole('button', { name: 'Codex Team Alpha' }))
    await userEvent.click(await documentScope.findByRole('link', { name: /去号池查看完整详情|Open in account pool/i }))
    await expect(documentScope.getByRole('heading', { name: /Codex Team Alpha/i })).toBeInTheDocument()
    await expect(documentScope.getByText(/账号 ID|ChatGPT account id/i)).toBeInTheDocument()
    await expect(documentScope.getByText(/org_alpha/i)).toBeInTheDocument()
  },
}


export const FastIndicatorStates: Story = {
  parameters: {
    docs: {
      description: {
        story:
          'Covers the fast indicator matrix: effective priority, requested-only fallback, requested priority with missing response tier, effective priority despite non-priority request, and a flex request with no lightning icon.',
      },
    },
  },
  args: {
    records: fastIndicatorRecords,
    isLoading: false,
    error: null,
  },
}

export const ReasoningEffortStates: Story = {
  parameters: {
    docs: {
      description: {
        story:
          'Matrix story for visually checking every reasoning effort state the table may show: `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, missing (`—`), and an unknown raw string. Supported API-level values were verified against the OpenAI API docs on 2026-03-07; actual model support remains model-dependent. The intended color ladder is neutral -> cool -> primary -> warning -> error, with unknown values rendered as dashed neutral badges.',
      },
    },
  },
  args: {
    records: reasoningEffortRecords,
    isLoading: false,
    error: null,
  },
}

export const Empty: Story = {
  parameters: {
    docs: {
      description: {
        story: 'Empty state used when the request succeeds but no invocations match the current filters.',
      },
    },
  },
  args: {
    records: [],
    isLoading: false,
    error: null,
  },
}

export const Loading: Story = {
  parameters: {
    docs: {
      description: {
        story: 'Loading placeholder used while invocation records are being fetched or refreshed.',
      },
    },
  },
  args: {
    records: [],
    isLoading: true,
    error: null,
  },
}

export const LoadError: Story = {
  parameters: {
    docs: {
      description: {
        story: 'Error banner state used when the invocation request fails and the user needs retry context.',
      },
    },
  },
  args: {
    records: [],
    isLoading: false,
    error: 'Request failed: 500 Internal Server Error',
  },
}
