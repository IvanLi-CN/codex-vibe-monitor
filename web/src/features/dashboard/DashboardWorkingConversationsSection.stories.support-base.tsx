import { type ReactNode, useLayoutEffect } from "react";
import { expect, userEvent, waitFor } from "storybook/test";
import type {
  ApiInvocation,
  PromptCacheConversation,
  PromptCacheConversationInvocationPreview,
  PromptCacheConversationsResponse,
  UpstreamAccountActivityResponse,
} from "../../lib/api";
import { useTheme } from "../../theme";
import { DASHBOARD_WORKSPACE_VIEW_STORAGE_KEY } from "./dashboardActivityRange";

function StorySurface({ children }: { children: ReactNode }) {
  return (
    <div className="min-h-screen bg-base-200 px-4 py-6 text-base-content sm:px-6">
      <div className="app-shell-boundary">{children}</div>
    </div>
  );
}

function DashboardAccountWindowEvidenceSurface({ children }: { children: ReactNode }) {
  return (
    <div
      data-visual-evidence-surface="dashboard-upstream-account-window"
      className="bg-base-200 p-10 text-base-content"
    >
      <div data-visual-evidence-target="dashboard-upstream-account-window-target">{children}</div>
    </div>
  );
}

const DASHBOARD_STORY_PROMPT_CACHE_BINDING_ACCOUNTS = [
  {
    id: 21,
    kind: "oauth_codex",
    provider: "codex",
    displayName: "growth.6vv4@relay.example",
    groupName: "CIII",
    status: "active",
    displayStatus: "active",
    enabled: true,
  },
  {
    id: 101,
    kind: "oauth_codex",
    provider: "codex",
    displayName: "Codex Pro - Tokyo",
    groupName: "Tokyo",
    status: "active",
    displayStatus: "active",
    enabled: true,
  },
  {
    id: 102,
    kind: "oauth_codex",
    provider: "codex",
    displayName: "ops-west@relay.example",
    groupName: "Relay-Blue",
    status: "active",
    displayStatus: "active",
    enabled: true,
  },
] as const;

function ForcedWorkspaceViewStory({
  view,
  children,
}: {
  view: "conversations" | "upstreamAccounts";
  children: ReactNode;
}) {
  if (typeof window !== "undefined") {
    window.localStorage.setItem(DASHBOARD_WORKSPACE_VIEW_STORAGE_KEY, view);
  }
  return <>{children}</>;
}

function useStoryTheme(theme?: "vibe-light" | "vibe-dark") {
  const { setThemeMode } = useTheme();

  useLayoutEffect(() => {
    if (!theme) return;
    const previousBodyTheme = document.body.getAttribute("data-theme");
    const previousBodyColorMode = document.body.getAttribute("data-color-mode");
    const nextThemeMode = theme === "vibe-dark" ? "dark" : "light";

    setThemeMode(nextThemeMode);
    document.body.setAttribute("data-theme", theme);
    document.body.setAttribute("data-color-mode", nextThemeMode);

    return () => {
      if (previousBodyTheme) {
        document.body.setAttribute("data-theme", previousBodyTheme);
      } else {
        document.body.removeAttribute("data-theme");
      }
      if (previousBodyColorMode) {
        document.body.setAttribute("data-color-mode", previousBodyColorMode);
      } else {
        document.body.removeAttribute("data-color-mode");
      }
    };
  }, [setThemeMode, theme]);
}

function readPerceptualChannel(color: string) {
  const match = color.match(/-?\d*\.?\d+/);
  return match ? Number.parseFloat(match[0]) : Number.NaN;
}

function requireFixture<T>(value: T | undefined): T {
  if (value === undefined) throw new Error("missing story fixture");
  return value;
}

function parseComputedColorChannels(color: string) {
  return Array.from(color.matchAll(/-?(?:\d*\.\d+|\d+)(?:e[+-]?\d+)?/gi), (match) =>
    Number.parseFloat(match[0]),
  );
}

function oklabRelativeLuminance(lightness: number, a: number, b: number) {
  const l = (lightness + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const m = (lightness - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s = (lightness - 0.0894841775 * a - 1.291485548 * b) ** 3;
  const red = 4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s;
  const green = -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s;
  const blue = -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s;

  return 0.2126 * red + 0.7152 * green + 0.0722 * blue;
}

function imageEditInkChroma(color: string) {
  const channels = parseComputedColorChannels(color);
  if (color.startsWith("oklch(") && channels.length >= 2) {
    return channels[1] ?? Number.NaN;
  }
  if (color.startsWith("oklab(") && channels.length >= 3) {
    return Math.hypot(channels[1] ?? Number.NaN, channels[2] ?? Number.NaN);
  }

  throw new Error(`Unsupported computed color: ${color}`);
}

function relativeLuminance(color: string) {
  const channels = parseComputedColorChannels(color);

  if (color.startsWith("oklab(") && channels.length >= 3) {
    const [lightness, a, b] = channels;
    return oklabRelativeLuminance(lightness ?? Number.NaN, a ?? Number.NaN, b ?? Number.NaN);
  }

  if (color.startsWith("oklch(") && channels.length >= 3) {
    const [lightness, chroma, hue] = channels;
    const hueRadians = ((hue ?? Number.NaN) * Math.PI) / 180;
    return oklabRelativeLuminance(
      lightness ?? Number.NaN,
      (chroma ?? Number.NaN) * Math.cos(hueRadians),
      (chroma ?? Number.NaN) * Math.sin(hueRadians),
    );
  }

  if (color.startsWith("rgb(") && channels.length >= 3) {
    const linear = channels.slice(0, 3).map((channel) => {
      const normalized = channel / 255;
      return normalized <= 0.04045 ? normalized / 12.92 : ((normalized + 0.055) / 1.055) ** 2.4;
    });

    const [red, green, blue] = linear;
    return (
      0.2126 * (red ?? Number.NaN) + 0.7152 * (green ?? Number.NaN) + 0.0722 * (blue ?? Number.NaN)
    );
  }

  throw new Error(`Unsupported computed color: ${color}`);
}

function contrastRatio(foreground: string, background: string) {
  const foregroundLuminance = relativeLuminance(foreground);
  const backgroundLuminance = relativeLuminance(background);
  const lighter = Math.max(foregroundLuminance, backgroundLuminance);
  const darker = Math.min(foregroundLuminance, backgroundLuminance);

  return (lighter + 0.05) / (darker + 0.05);
}

async function openBulkClearBindingDialog(canvasElement: HTMLElement) {
  await selectConversationForBulkActions(canvasElement);

  const routeBindButton = canvasElement.ownerDocument.body.querySelector(
    '[data-testid="dashboard-working-conversations-route-bind-button"]',
  );
  if (!(routeBindButton instanceof HTMLButtonElement)) {
    throw new Error("missing route bind button");
  }

  await userEvent.click(routeBindButton);

  await waitFor(() => {
    const routeBindDialog = canvasElement.ownerDocument.body.querySelector(
      '[data-testid="dashboard-working-conversations-route-bind-dialog"]',
    );
    expect(routeBindDialog).not.toBeNull();
  });

  const clearButton = canvasElement.ownerDocument.body.querySelector(
    '[data-testid="dashboard-working-conversations-route-bind-clear-button"]',
  );
  if (!(clearButton instanceof HTMLButtonElement)) {
    throw new Error("missing route bind clear button");
  }

  await userEvent.click(clearButton);

  await waitFor(() => {
    const dialog = canvasElement.ownerDocument.body.querySelector(
      '[data-testid="dashboard-working-conversations-clear-binding-dialog"]',
    );
    expect(dialog).not.toBeNull();
    expect(dialog?.textContent).toContain("清空绑定");
    expect(dialog?.textContent).not.toContain("重选");
  });

  const dialog = canvasElement.ownerDocument.body.querySelector(
    '[data-testid="dashboard-working-conversations-clear-binding-dialog"]',
  );
  if (!(dialog instanceof HTMLElement)) {
    throw new Error("missing clear binding dialog");
  }

  const footer = dialog.querySelectorAll(".dialog-chrome-surface")[1];
  const callout = dialog.querySelector(".destructive-callout-surface");
  if (!(footer instanceof HTMLElement) || !(callout instanceof HTMLElement)) {
    throw new Error("missing themed destructive surfaces");
  }

  return { dialog, footer, callout };
}

async function enableConversationSelectionMode(canvasElement: HTMLElement) {
  const selectionModeButton = canvasElement.querySelector(
    '[data-testid="dashboard-working-conversations-selection-mode-button"]',
  );
  if (!(selectionModeButton instanceof HTMLButtonElement)) {
    throw new Error("missing selection mode button");
  }

  await userEvent.click(selectionModeButton);
  await waitFor(() => {
    const firstCard = canvasElement.querySelector<HTMLElement>(
      '[data-testid="dashboard-working-conversation-card"]',
    );
    expect(firstCard?.getAttribute("data-selection-mode")).toBe("true");
  });
}

async function selectConversationForBulkActions(canvasElement: HTMLElement) {
  await enableConversationSelectionMode(canvasElement);

  const firstCard = canvasElement.querySelector<HTMLElement>(
    '[data-testid="dashboard-working-conversation-card"]',
  );
  if (!(firstCard instanceof HTMLElement)) {
    throw new Error("missing selectable conversation card");
  }

  await userEvent.click(firstCard);
  await waitFor(() => {
    const panel = canvasElement.ownerDocument.body.querySelector(
      '[data-testid="dashboard-working-conversations-bulk-panel"]',
    );
    expect(panel).not.toBeNull();
    expect(panel?.textContent).toContain("已选 1 个对话");
  });
}

function jsonResponse(payload: unknown, status = 200) {
  return new Response(JSON.stringify(payload), {
    status,
    headers: {
      "Content-Type": "application/json",
    },
  });
}

function createPreview(
  overrides: Partial<PromptCacheConversationInvocationPreview> & {
    id: number;
    invokeId: string;
    occurredAt: string;
    status: string;
  },
): PromptCacheConversationInvocationPreview {
  return {
    id: overrides.id,
    invokeId: overrides.invokeId,
    promptCacheKey: "promptCacheKey" in overrides ? (overrides.promptCacheKey ?? null) : null,
    occurredAt: overrides.occurredAt,
    status: overrides.status,
    livePhase: overrides.livePhase ?? null,
    failureClass: overrides.failureClass ?? "none",
    routeMode: overrides.routeMode ?? "pool",
    model: overrides.model ?? "gpt-5.4",
    requestModel: "requestModel" in overrides ? (overrides.requestModel ?? null) : "gpt-5.4",
    responseModel:
      "responseModel" in overrides
        ? (overrides.responseModel ?? null)
        : (overrides.model ?? "gpt-5.4"),
    totalTokens: overrides.totalTokens ?? 240,
    cost: overrides.cost ?? 0.0182,
    proxyDisplayName:
      "proxyDisplayName" in overrides ? (overrides.proxyDisplayName ?? null) : "tokyo-edge-01",
    upstreamAccountId:
      "upstreamAccountId" in overrides ? (overrides.upstreamAccountId ?? null) : 42,
    upstreamAccountName:
      "upstreamAccountName" in overrides
        ? (overrides.upstreamAccountName ?? null)
        : "pool-alpha@example.com",
    upstreamAccountPlanType:
      "upstreamAccountPlanType" in overrides
        ? (overrides.upstreamAccountPlanType ?? null)
        : undefined,
    endpoint: overrides.endpoint ?? "/v1/responses",
    compactionRequestKind: overrides.compactionRequestKind ?? null,
    compactionResponseKind: overrides.compactionResponseKind ?? null,
    imageIntent: overrides.imageIntent ?? null,
    transport: overrides.transport,
    source: overrides.source ?? "pool",
    inputTokens: overrides.inputTokens ?? 148,
    outputTokens: overrides.outputTokens ?? 92,
    cacheInputTokens: overrides.cacheInputTokens ?? 36,
    reasoningTokens: overrides.reasoningTokens ?? 24,
    reasoningEffort: "reasoningEffort" in overrides ? overrides.reasoningEffort : "high",
    errorMessage: overrides.errorMessage,
    failureKind: overrides.failureKind,
    isActionable: overrides.isActionable,
    responseContentEncoding: overrides.responseContentEncoding ?? "gzip",
    requestedServiceTier: overrides.requestedServiceTier ?? "priority",
    serviceTier: overrides.serviceTier ?? "priority",
    tReqReadMs: overrides.tReqReadMs ?? 14,
    tReqParseMs: overrides.tReqParseMs ?? 8,
    tUpstreamConnectMs: overrides.tUpstreamConnectMs ?? 136,
    tUpstreamTtfbMs: "tUpstreamTtfbMs" in overrides ? (overrides.tUpstreamTtfbMs ?? null) : 98,
    firstTokenMs: "firstTokenMs" in overrides ? (overrides.firstTokenMs ?? null) : 742,
    tUpstreamStreamMs:
      "tUpstreamStreamMs" in overrides ? (overrides.tUpstreamStreamMs ?? null) : 324,
    tRespParseMs: overrides.tRespParseMs ?? 12,
    tPersistMs: overrides.tPersistMs ?? 9,
    tTotalMs: overrides.tTotalMs ?? 601,
  };
}

function isInFlightStatus(status: string | null | undefined) {
  const normalized = status?.trim().toLowerCase() ?? "";
  return normalized === "running" || normalized === "pending";
}

function createConversation(
  promptCacheKey: string,
  recentInvocations: PromptCacheConversationInvocationPreview[],
  overrides: Partial<PromptCacheConversation> = {},
): PromptCacheConversation {
  const lastInFlightPreview = recentInvocations.find((preview) => isInFlightStatus(preview.status));
  const lastTerminalPreview = recentInvocations.find(
    (preview) => !isInFlightStatus(preview.status),
  );
  return {
    promptCacheKey,
    hasEncryptedSessionOwner: overrides.hasEncryptedSessionOwner ?? false,
    encryptedOwnerAccountId: overrides.encryptedOwnerAccountId ?? null,
    encryptedOwnerAccountName: overrides.encryptedOwnerAccountName ?? null,
    encryptedOwnerGroupName: overrides.encryptedOwnerGroupName ?? null,
    requestCount: overrides.requestCount ?? recentInvocations.length,
    totalTokens:
      overrides.totalTokens ??
      recentInvocations.reduce((sum, preview) => sum + Math.max(0, preview.totalTokens), 0),
    totalCost:
      overrides.totalCost ??
      Number(recentInvocations.reduce((sum, preview) => sum + (preview.cost ?? 0), 0).toFixed(4)),
    createdAt:
      overrides.createdAt ??
      recentInvocations[recentInvocations.length - 1]?.occurredAt ??
      "2026-04-04T10:00:00Z",
    lastActivityAt:
      overrides.lastActivityAt ?? recentInvocations[0]?.occurredAt ?? "2026-04-04T10:00:00Z",
    lastTerminalAt: overrides.lastTerminalAt ?? lastTerminalPreview?.occurredAt ?? null,
    lastInFlightAt: overrides.lastInFlightAt ?? lastInFlightPreview?.occurredAt ?? null,
    inFlightPhaseCounts: overrides.inFlightPhaseCounts ?? {
      queued: 0,
      requesting: 0,
      responding: 0,
    },
    cursor: overrides.cursor ?? promptCacheKey,
    manualBinding: overrides.manualBinding ?? null,
    upstreamAccounts: overrides.upstreamAccounts ?? [],
    recentInvocations,
    last24hRequests: overrides.last24hRequests ?? [],
  };
}

function createResponse(
  conversations: PromptCacheConversation[],
): PromptCacheConversationsResponse {
  return {
    rangeStart: "2026-04-04T10:00:00Z",
    rangeEnd: "2026-04-04T10:05:00Z",
    selectionMode: "activityWindow",
    selectedLimit: null,
    selectedActivityHours: null,
    selectedActivityMinutes: 5,
    implicitFilter: { kind: null, filteredCount: 0 },
    conversations,
  };
}

function createRelativeStoryIso(offsetMs: number) {
  return new Date(Date.now() + offsetMs).toISOString();
}

function buildRecordFromPreview(preview: PromptCacheConversationInvocationPreview): ApiInvocation {
  return {
    id: preview.id,
    invokeId: preview.invokeId,
    promptCacheKey: preview.promptCacheKey ?? undefined,
    occurredAt: preview.occurredAt,
    createdAt: preview.occurredAt,
    source: preview.source ?? "pool",
    routeMode: preview.routeMode ?? "pool",
    proxyDisplayName: preview.proxyDisplayName ?? undefined,
    upstreamAccountId: preview.upstreamAccountId ?? null,
    upstreamAccountName: preview.upstreamAccountName ?? undefined,
    endpoint: preview.endpoint ?? undefined,
    compactionRequestKind: preview.compactionRequestKind ?? null,
    compactionResponseKind: preview.compactionResponseKind ?? null,
    imageIntent: preview.imageIntent ?? null,
    transport: preview.transport,
    model: preview.model ?? undefined,
    requestModel: preview.requestModel ?? undefined,
    responseModel: preview.responseModel ?? undefined,
    status: preview.status,
    inputTokens: preview.inputTokens,
    outputTokens: preview.outputTokens,
    cacheInputTokens: preview.cacheInputTokens,
    reasoningTokens: preview.reasoningTokens,
    reasoningEffort: preview.reasoningEffort,
    totalTokens: preview.totalTokens,
    cost: preview.cost ?? undefined,
    errorMessage: preview.errorMessage,
    failureKind: preview.failureKind,
    failureClass: preview.failureClass ?? undefined,
    isActionable: preview.isActionable,
    responseContentEncoding: preview.responseContentEncoding ?? undefined,
    requestedServiceTier: preview.requestedServiceTier ?? undefined,
    serviceTier: preview.serviceTier ?? undefined,
    tReqReadMs: preview.tReqReadMs,
    tReqParseMs: preview.tReqParseMs,
    tUpstreamConnectMs: preview.tUpstreamConnectMs,
    tUpstreamTtfbMs: preview.tUpstreamTtfbMs,
    firstTokenMs: preview.firstTokenMs,
    tUpstreamStreamMs: preview.tUpstreamStreamMs,
    tRespParseMs: preview.tRespParseMs,
    tPersistMs: preview.tPersistMs,
    tTotalMs: preview.tTotalMs,
  };
}

const UPSTREAM_ACCOUNT_ACTIVITY_WINDOW_MINUTES = 5;

const BASE_UPSTREAM_ACCOUNT_RECENT_INVOCATION_SEEDS = [
  {
    promptCacheKey: "story-account-1",
    invokeId: "acct-invoke-1",
    occurredAt: "2026-04-04T10:05:00Z",
    status: "success",
    model: "gpt-5.5",
    requestModel: "gpt-5.5-mini",
    responseModel: "gpt-5.5",
    imageIntent: "yes" as const,
    totalTokens: 18_240,
    cost: 0.2744,
    inputTokens: 7_860,
    outputTokens: 5_120,
    cacheInputTokens: 4_380,
    reasoningTokens: 880,
    tUpstreamConnectMs: 182,
    tUpstreamTtfbMs: 1_180,
    tUpstreamStreamMs: 23_740,
    tTotalMs: 26_410,
    requestedServiceTier: "priority",
    serviceTier: "priority",
  },
  {
    promptCacheKey: "story-account-2",
    invokeId: "acct-invoke-2",
    occurredAt: "2026-04-04T10:04:18Z",
    status: "running",
    livePhase: "responding" as const,
    model: "gpt-5.5",
    requestModel: "gpt-5.5-mini",
    responseModel: "gpt-5.5",
    totalTokens: 11_620,
    cost: 0.1682,
    inputTokens: 4_940,
    outputTokens: 3_720,
    cacheInputTokens: 2_360,
    reasoningTokens: 600,
    tUpstreamConnectMs: 164,
    tUpstreamTtfbMs: 920,
    firstTokenMs: 1_340,
    tUpstreamStreamMs: 11_480,
    tTotalMs: 13_970,
    requestedServiceTier: "priority",
    serviceTier: "priority",
  },
  {
    promptCacheKey: "story-account-3",
    invokeId: "acct-invoke-3",
    occurredAt: "2026-04-04T10:03:42Z",
    status: "failed",
    failureClass: "service_failure" as const,
    failureKind: "upstream_http_429" as const,
    errorMessage: "upstream returned 429 after the fallback lane exhausted its retry budget",
    model: "gpt-5.5-mini",
    requestModel: "gpt-5.5-mini",
    responseModel: "gpt-5.5-mini",
    totalTokens: 2_960,
    cost: 0.0412,
    inputTokens: 1_840,
    outputTokens: 320,
    cacheInputTokens: 640,
    reasoningTokens: 160,
    tUpstreamConnectMs: 148,
    tUpstreamTtfbMs: 2_460,
    tUpstreamStreamMs: 8_140,
    tTotalMs: 12_340,
    requestedServiceTier: "standard",
    serviceTier: "standard",
  },
  {
    promptCacheKey: "story-account-4",
    invokeId: "acct-invoke-4",
    occurredAt: "2026-04-04T10:02:55Z",
    status: "success",
    model: "gpt-5.5-mini",
    requestModel: "gpt-5.5-mini",
    responseModel: "gpt-5.5-mini",
    totalTokens: 8_420,
    cost: 0.1198,
    inputTokens: 3_920,
    outputTokens: 2_140,
    cacheInputTokens: 1_980,
    reasoningTokens: 380,
    tUpstreamConnectMs: 128,
    tUpstreamTtfbMs: 610,
    tUpstreamStreamMs: 5_940,
    tTotalMs: 7_280,
    requestedServiceTier: "standard",
    serviceTier: "standard",
  },
  {
    promptCacheKey: "story-account-5",
    invokeId: "acct-invoke-5",
    occurredAt: "2026-04-04T10:02:07Z",
    status: "pending",
    livePhase: "queued" as const,
    model: "gpt-5.5-mini",
    requestModel: "gpt-5.5-mini",
    responseModel: null,
    totalTokens: 0,
    cost: 0,
    inputTokens: 0,
    outputTokens: 0,
    cacheInputTokens: 0,
    reasoningTokens: 0,
    tUpstreamConnectMs: 0,
    tUpstreamTtfbMs: null,
    tUpstreamStreamMs: null,
    tTotalMs: 0,
    requestedServiceTier: "priority",
    serviceTier: undefined,
  },
  {
    promptCacheKey: "story-account-6",
    invokeId: "acct-invoke-6",
    occurredAt: "2026-04-04T10:01:34Z",
    status: "success",
    model: "gpt-5.5",
    requestModel: "gpt-5.5",
    responseModel: "gpt-5.5",
    totalTokens: 14_180,
    cost: 0.2116,
    inputTokens: 6_420,
    outputTokens: 4_360,
    cacheInputTokens: 2_780,
    reasoningTokens: 620,
    tUpstreamConnectMs: 154,
    tUpstreamTtfbMs: 1_020,
    tUpstreamStreamMs: 17_680,
    tTotalMs: 19_940,
    requestedServiceTier: "priority",
    serviceTier: "priority",
  },
  {
    promptCacheKey: "story-account-7",
    invokeId: "acct-invoke-7",
    occurredAt: "2026-04-04T10:01:02Z",
    status: "success",
    model: "gpt-5.5-mini",
    requestModel: "gpt-5.5-mini",
    responseModel: "gpt-5.5-mini",
    totalTokens: 6_780,
    cost: 0.0914,
    inputTokens: 3_020,
    outputTokens: 1_940,
    cacheInputTokens: 1_500,
    reasoningTokens: 320,
    tUpstreamConnectMs: 116,
    tUpstreamTtfbMs: 540,
    tUpstreamStreamMs: 4_120,
    tTotalMs: 5_380,
    requestedServiceTier: "standard",
    serviceTier: "standard",
  },
  {
    promptCacheKey: "story-account-8",
    invokeId: "acct-invoke-8",
    occurredAt: "2026-04-04T10:00:41Z",
    status: "failed",
    failureClass: "service_failure" as const,
    failureKind: "upstream_http_5xx" as const,
    errorMessage: "upstream returned 502 while compact lane was streaming the response body",
    model: "gpt-5.5",
    requestModel: "gpt-5.5-mini",
    responseModel: "gpt-5.5",
    totalTokens: 4_320,
    cost: 0.0631,
    inputTokens: 2_160,
    outputTokens: 640,
    cacheInputTokens: 1_200,
    reasoningTokens: 320,
    tUpstreamConnectMs: 132,
    tUpstreamTtfbMs: 3_120,
    tUpstreamStreamMs: 9_840,
    tTotalMs: 13_460,
    requestedServiceTier: "priority",
    serviceTier: "priority",
  },
  {
    promptCacheKey: "story-account-9",
    invokeId: "acct-invoke-9",
    occurredAt: "2026-04-04T10:00:19Z",
    status: "running",
    livePhase: "requesting" as const,
    model: "gpt-5.5",
    requestModel: "gpt-5.5",
    responseModel: null,
    totalTokens: 1_920,
    cost: 0.0264,
    inputTokens: 1_280,
    outputTokens: 0,
    cacheInputTokens: 520,
    reasoningTokens: 120,
    tUpstreamConnectMs: 188,
    tUpstreamTtfbMs: null,
    tUpstreamStreamMs: null,
    tTotalMs: 1_940,
    requestedServiceTier: "priority",
    serviceTier: undefined,
  },
  {
    promptCacheKey: "story-account-10",
    invokeId: "acct-invoke-10",
    occurredAt: "2026-04-04T09:59:51Z",
    status: "success",
    model: "gpt-5.5-mini",
    requestModel: "gpt-5.5-mini",
    responseModel: "gpt-5.5-mini",
    totalTokens: 5_440,
    cost: 0.0738,
    inputTokens: 2_520,
    outputTokens: 1_640,
    cacheInputTokens: 1_040,
    reasoningTokens: 240,
    tUpstreamConnectMs: 106,
    tUpstreamTtfbMs: 460,
    tUpstreamStreamMs: 3_820,
    tTotalMs: 4_860,
    requestedServiceTier: "standard",
    serviceTier: "standard",
  },
  {
    promptCacheKey: "story-account-11",
    invokeId: "acct-invoke-11",
    occurredAt: "2026-04-04T09:59:24Z",
    status: "success",
    model: "gpt-5.5",
    requestModel: "gpt-5.5",
    responseModel: "gpt-5.5",
    totalTokens: 22_860,
    cost: 0.3362,
    inputTokens: 10_120,
    outputTokens: 6_840,
    cacheInputTokens: 4_920,
    reasoningTokens: 980,
    tUpstreamConnectMs: 170,
    tUpstreamTtfbMs: 1_360,
    tUpstreamStreamMs: 29_420,
    tTotalMs: 31_840,
    requestedServiceTier: "priority",
    serviceTier: "priority",
  },
  {
    promptCacheKey: "story-account-12",
    invokeId: "acct-invoke-12",
    occurredAt: "2026-04-04T09:58:57Z",
    status: "success",
    model: "gpt-5.5-mini",
    requestModel: "gpt-5.5-mini",
    responseModel: "gpt-5.5-mini",
    totalTokens: 7_260,
    cost: 0.0984,
    inputTokens: 3_140,
    outputTokens: 2_180,
    cacheInputTokens: 1_600,
    reasoningTokens: 340,
    tUpstreamConnectMs: 118,
    tUpstreamTtfbMs: 590,
    tUpstreamStreamMs: 4_680,
    tTotalMs: 5_920,
    requestedServiceTier: "standard",
    serviceTier: "standard",
  },
  {
    promptCacheKey: "story-account-13",
    invokeId: "acct-invoke-13",
    occurredAt: "2026-04-04T09:58:29Z",
    status: "failed",
    failureClass: "service_failure" as const,
    failureKind: "upstream_http_408" as const,
    errorMessage: "upstream compact lane timed out after the first byte during a long stream",
    model: "gpt-5.5",
    requestModel: "gpt-5.5-mini",
    responseModel: "gpt-5.5",
    totalTokens: 3_680,
    cost: 0.0528,
    inputTokens: 1_920,
    outputTokens: 480,
    cacheInputTokens: 1_040,
    reasoningTokens: 240,
    tUpstreamConnectMs: 140,
    tUpstreamTtfbMs: 2_880,
    tUpstreamStreamMs: 10_260,
    tTotalMs: 14_180,
    requestedServiceTier: "priority",
    serviceTier: "priority",
  },
  {
    promptCacheKey: "story-account-14",
    invokeId: "acct-invoke-14",
    occurredAt: "2026-04-04T09:58:03Z",
    status: "success",
    model: "gpt-5.5",
    requestModel: "gpt-5.5",
    responseModel: "gpt-5.5",
    totalTokens: 16_320,
    cost: 0.2418,
    inputTokens: 7_120,
    outputTokens: 4_980,
    cacheInputTokens: 3_460,
    reasoningTokens: 760,
    tUpstreamConnectMs: 162,
    tUpstreamTtfbMs: 1_140,
    tUpstreamStreamMs: 18_460,
    tTotalMs: 21_120,
    requestedServiceTier: "priority",
    serviceTier: "priority",
  },
  {
    promptCacheKey: "story-account-15",
    invokeId: "acct-invoke-15",
    occurredAt: "2026-04-04T09:57:35Z",
    status: "success",
    model: "gpt-5.5-mini",
    requestModel: "gpt-5.5-mini",
    responseModel: "gpt-5.5-mini",
    totalTokens: 9_140,
    cost: 0.1246,
    inputTokens: 4_120,
    outputTokens: 2_740,
    cacheInputTokens: 1_860,
    reasoningTokens: 420,
    tUpstreamConnectMs: 124,
    tUpstreamTtfbMs: 640,
    tUpstreamStreamMs: 6_280,
    tTotalMs: 7_560,
    requestedServiceTier: "standard",
    serviceTier: "standard",
  },
  {
    promptCacheKey: "story-account-16",
    invokeId: "acct-invoke-16",
    occurredAt: "2026-04-04T09:57:12Z",
    status: "success",
    model: "gpt-5.5",
    requestModel: "gpt-5.5-mini",
    responseModel: "gpt-5.5",
    totalTokens: 12_880,
    cost: 0.1848,
    inputTokens: 5_940,
    outputTokens: 3_740,
    cacheInputTokens: 2_620,
    reasoningTokens: 580,
    tUpstreamConnectMs: 150,
    tUpstreamTtfbMs: 920,
    tUpstreamStreamMs: 12_780,
    tTotalMs: 14_920,
    requestedServiceTier: "priority",
    serviceTier: "priority",
  },
] satisfies Array<
  Partial<PromptCacheConversationInvocationPreview> & {
    promptCacheKey: string;
    invokeId: string;
    status: string;
  }
>;

const BASE_UPSTREAM_ACCOUNT_RECENT_INVOCATION_OFFSETS_MS = [
  -18_000, -52_000, -96_000, -143_000, -188_000, -234_000, -278_000, -321_000, -366_000, -411_000,
  -452_000, -497_000, -543_000, -587_000, -632_000, -676_000,
];

const BASE_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN = {
  cacheWriteTokens: 73_600,
  cacheReadTokens: 39_800,
  outputTokens: 73_000,
  costs: {
    input: 1.12,
    cacheWrite: 0.61,
    cacheRead: 0.28,
    output: 1.47,
    reasoning: 0.37,
    unknown: 0,
  },
  models: [
    {
      model: "gpt-5.5",
      reasoningEffort: "high" as const,
      cacheWriteTokens: 45_000,
      cacheReadTokens: 24_200,
      outputTokens: 44_800,
      costs: {
        input: 0.71,
        cacheWrite: 0.38,
        cacheRead: 0.19,
        output: 0.86,
        reasoning: 0.22,
        unknown: 0,
      },
    },
    {
      model: "gpt-5.5-mini",
      reasoningEffort: "—",
      cacheWriteTokens: 28_600,
      cacheReadTokens: 15_600,
      outputTokens: 28_200,
      costs: {
        input: 0.41,
        cacheWrite: 0.23,
        cacheRead: 0.09,
        output: 0.61,
        reasoning: 0.15,
        unknown: 0,
      },
    },
  ],
};

const ADAPTIVE_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN = {
  cacheWriteTokens: 2_584_200,
  cacheReadTokens: 1_238_540,
  outputTokens: 2_800_975,
  costs: {
    input: 88.16,
    cacheWrite: 43.28,
    cacheRead: 19.64,
    output: 92.41,
    reasoning: 31.07,
    unknown: 0,
  },
  models: [
    {
      model: "gpt-5.5",
      reasoningEffort: "high" as const,
      cacheWriteTokens: 1_648_100,
      cacheReadTokens: 804_240,
      outputTokens: 1_745_320,
      costs: {
        input: 56.22,
        cacheWrite: 27.56,
        cacheRead: 12.41,
        output: 58.92,
        reasoning: 19.23,
        unknown: 0,
      },
    },
    {
      model: "gpt-5.5-mini",
      reasoningEffort: "—",
      cacheWriteTokens: 936_100,
      cacheReadTokens: 434_300,
      outputTokens: 1_055_655,
      costs: {
        input: 31.94,
        cacheWrite: 15.72,
        cacheRead: 7.23,
        output: 33.49,
        reasoning: 11.84,
        unknown: 0,
      },
    },
  ],
};

function buildUpstreamAccountRecentInvocations(recentInvocationCount: number) {
  return BASE_UPSTREAM_ACCOUNT_RECENT_INVOCATION_SEEDS.slice(
    0,
    Math.max(
      0,
      Math.min(recentInvocationCount, BASE_UPSTREAM_ACCOUNT_RECENT_INVOCATION_SEEDS.length),
    ),
  ).map((seed, index) =>
    createPreview({
      id: 9901 + index,
      upstreamAccountId: 42,
      upstreamAccountName: "Pool Alpha",
      upstreamAccountPlanType: "enterprise",
      proxyDisplayName: index % 2 === 0 ? "tokyo-edge-01" : "singapore-edge-02",
      ...seed,
      occurredAt: createRelativeStoryIso(
        BASE_UPSTREAM_ACCOUNT_RECENT_INVOCATION_OFFSETS_MS[index] ?? -((index + 1) * 45_000),
      ),
    }),
  );
}

function perMinuteRate(total: number) {
  return Number((total / UPSTREAM_ACCOUNT_ACTIVITY_WINDOW_MINUTES).toFixed(2));
}

function buildUpstreamAccountActivityRoutingRule(
  overrides: Partial<
    NonNullable<UpstreamAccountActivityResponse["accounts"][number]["effectiveRoutingRule"]>
  >,
) {
  return {
    allowCutOut: true,
    allowCutIn: false,
    priorityTier: "no_new" as const,
    fastModeRewriteMode: "force_add" as const,
    imageToolRewriteMode: "keep_original" as const,
    concurrencyLimit: 3,
    upstream429RetryEnabled: true,
    upstream429MaxRetries: 2,
    ...overrides,
    availableModels: [],
    availableModelsDefined: false,
    systemDeniedModels: [],
    sourceTagIds: [],
    sourceTagNames: [],
    fieldSources: {
      allowCutOut: "root" as const,
      allowCutIn: "account" as const,
      priorityTier: "group" as const,
      fastModeRewriteMode: "account" as const,
      imageToolRewriteMode: "root" as const,
      concurrencyLimit: "group" as const,
      upstream429Retry: "group" as const,
      availableModels: "root" as const,
      systemDeniedModels: "root" as const,
    },
    timeouts: {
      responsesFirstByteTimeoutSecs: 120,
      compactFirstByteTimeoutSecs: 120,
      responsesStreamTimeoutSecs: 600,
      compactStreamTimeoutSecs: 600,
    },
    timeoutFieldSources: {
      responsesFirstByteTimeoutSecs: "root" as const,
      compactFirstByteTimeoutSecs: "root" as const,
      responsesStreamTimeoutSecs: "root" as const,
      compactStreamTimeoutSecs: "root" as const,
    },
  };
}

function createUpstreamAccountActivityStoryResponse(
  recentInvocationCount = 4,
  routingRuleOverrides: Partial<
    NonNullable<UpstreamAccountActivityResponse["accounts"][number]["effectiveRoutingRule"]>
  > = {},
): UpstreamAccountActivityResponse {
  const recentInvocations = buildUpstreamAccountRecentInvocations(recentInvocationCount);
  const totalTokens = 186_400;
  const totalCost = 3.85;
  const rangeEnd = createRelativeStoryIso(0);
  const rangeStart = createRelativeStoryIso(-UPSTREAM_ACCOUNT_ACTIVITY_WINDOW_MINUTES * 60_000);
  const latestConversationCreatedAt = createRelativeStoryIso(-252_000);
  const lastInvocationAt = recentInvocations[0]?.occurredAt ?? rangeEnd;
  return {
    range: "today",
    rangeStart,
    rangeEnd,
    accounts: [
      {
        upstreamAccountId: 42,
        displayName: "Pool Alpha",
        latestConversationCreatedAt,
        lastInvocationAt,
        groupName: "Primary",
        planType: "enterprise",
        enabled: true,
        displayStatus: "upstream_rejected",
        enableStatus: "enabled",
        workStatus: "rate_limited",
        healthStatus: "upstream_rejected",
        syncState: "idle",
        lastError: "fallback lane hit upstream 429 and then a compact stream retry ended with 502",
        lastActionReasonMessage: "峰值时段触发限流与上游拒绝，系统已回落到保守路由策略",
        requestCount: 32,
        successCount: 24,
        failureCount: 5,
        nonSuccessCount: 8,
        totalTokens,
        successTokens: 152_080,
        nonSuccessTokens: 34_320,
        failureTokens: 15_840,
        failureCost: 0.78,
        totalCost,
        usageBreakdown: BASE_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
        cacheHitRate: 0.34,
        tokensPerMinute: perMinuteRate(totalTokens),
        spendRate: perMinuteRate(totalCost),
        firstByteAvgMs: 480,
        firstTokenAvgMs: 4_380,
        avgTotalMs: 18_420,
        currentFirstTokenAvgMs: 4_380,
        currentAvgTotalMs: 18_420,
        inProgressInvocationCount: 3,
        inProgressPhaseCounts: {
          queued: 1,
          requesting: 1,
          responding: 1,
        },
        retryInvocationCount: 2,
        uploadBytesPerSecond: 46 * 1024,
        downloadBytesPerSecond: 214 * 1024,
        effectiveRoutingRule: buildUpstreamAccountActivityRoutingRule(routingRuleOverrides),
        recentInvocations,
      },
    ],
  };
}

function createImageEditEndpointUpstreamAccountActivityStoryResponse() {
  const response = createUpstreamAccountActivityStoryResponse();
  const account = response.accounts[0];
  const firstRecentInvocation = account?.recentInvocations[0];
  if (!account || !firstRecentInvocation) return response;

  account.recentInvocations = [
    {
      ...firstRecentInvocation,
      endpoint: "/v1/images/edits",
      imageIntent: "direct_image",
      model: "gpt-image-1",
      requestModel: "gpt-image-1",
      responseModel: "gpt-image-1",
    },
  ];

  return response;
}

function createUpstreamAccountAdaptiveMetricsStoryResponse() {
  const response = createUpstreamAccountActivityStoryResponse();
  const account = response.accounts[0];
  if (!account) return response;

  account.requestCount = 10_376;
  account.successCount = 10_233;
  account.failureCount = 143;
  account.nonSuccessCount = 143;
  account.uploadBytesPerSecond = 42 * 1024;
  account.downloadBytesPerSecond = 168 * 1024;
  account.totalTokens = 30_030_779;
  account.successTokens = 10_962_028;
  account.nonSuccessTokens = 19_068_751;
  account.failureTokens = 19_068_751;
  account.failureCost = 19_068.75;
  account.totalCost = 30_030_779.25;
  account.usageBreakdown = ADAPTIVE_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN;
  account.tokensPerMinute = 1_324_743;
  account.spendRate = 54;
  account.firstTokenAvgMs = 65_000;
  account.avgTotalMs = 3_600_000;
  account.currentFirstTokenAvgMs = 65_000;
  account.currentAvgTotalMs = 3_600_000;
  account.inProgressInvocationCount = 9;
  account.inProgressPhaseCounts = {
    queued: 2,
    requesting: 3,
    responding: 4,
  };
  account.retryInvocationCount = 9;
  account.cacheHitRate = 0.41;

  return response;
}

const LONG_ERROR_SUMMARY =
  '[upstream_http_5xx] pool upstream responded with 502: {"error":{"message":"Upstream request failed","type":"upstream_error"}} event: response.failed data: {"type":"response.failed","response":{"id":"resp_story_error_summary","model":"gpt-5.4","status":"failed"}}';

const currentAndPreviousResponse = createResponse([
  createConversation("pck-current-previous", [
    createPreview({
      id: 12,
      invokeId: "invoke-12",
      occurredAt: "2026-04-04T10:04:20Z",
      status: "completed",
      upstreamAccountName: "growth-alpha@example.com",
      upstreamAccountPlanType: "plus",
      reasoningEffort: "medium",
      imageIntent: "yes",
      cacheInputTokens: 144,
      cost: 0.2,
      tTotalMs: 20_000,
    }),
    createPreview({
      id: 11,
      invokeId: "invoke-11",
      occurredAt: "2026-04-04T10:01:12Z",
      status: "completed",
      model: "gpt-5.4-mini",
      upstreamAccountName: "backup-alpha@example.com",
      upstreamAccountPlanType: "free",
      requestedServiceTier: "auto",
      serviceTier: "auto",
    }),
    createPreview({
      id: 10,
      invokeId: "invoke-10",
      occurredAt: "2026-04-04T09:58:44Z",
      status: "completed",
      model: "gpt-5.4-long-context-preview",
      upstreamAccountName: "backup-alpha-long-account-label-for-truncation@example.com",
      upstreamAccountPlanType: "team",
    }),
  ]),
]);

const currentOnlyResponse = createResponse([
  createConversation("pck-placeholder-only", [
    createPreview({
      id: 21,
      invokeId: "invoke-21",
      occurredAt: "2026-04-04T10:04:42Z",
      status: "completed",
      upstreamAccountName: "warmup-alpha@example.com",
    }),
  ]),
]);

const imageEndpointChipResponse = createResponse([
  createConversation("pck-image-endpoint-chip", [
    createPreview({
      id: 31,
      invokeId: "invoke-image-endpoint-chip",
      occurredAt: "2026-04-04T10:05:42Z",
      status: "completed",
      endpoint: "/v1/images/generations",
      imageIntent: "yes",
      model: "gpt-image-1",
      upstreamAccountName: "image-alpha@example.com",
      upstreamAccountPlanType: "team",
    }),
    createPreview({
      id: 30,
      invokeId: "invoke-image-endpoint-previous",
      occurredAt: "2026-04-04T10:03:18Z",
      status: "completed",
      endpoint: "/v1/images/variations",
      model: "gpt-image-1",
      upstreamAccountName: "image-backup@example.com",
      upstreamAccountPlanType: "free",
    }),
  ]),
]);

const warningSuccessConversationResponse = createResponse([
  createConversation("pck-warning-success", [
    createPreview({
      id: 61,
      invokeId: "invoke-warning-success",
      occurredAt: "2026-04-04T10:04:20Z",
      status: "warning_success",
      failureKind: "downstream_closed",
      failureClass: "none",
      errorMessage: "[downstream_closed] downstream closed while streaming upstream response",
      upstreamAccountId: 42,
      upstreamAccountName: "Pool Alpha",
      totalTokens: 167_710,
      cost: 0.0629,
      tUpstreamTtfbMs: 1_131,
      tUpstreamStreamMs: 15_849,
      tTotalMs: 16_980,
    }),
    createPreview({
      id: 60,
      invokeId: "invoke-warning-success-previous",
      occurredAt: "2026-04-04T10:01:12Z",
      status: "completed",
      upstreamAccountName: "Pool Alpha",
      model: "gpt-5.4-mini",
    }),
  ]),
]);

function createRunningOnlyResponse() {
  return createResponse([
    createConversation("pck-running-only", [
      createPreview({
        id: 31,
        invokeId: "invoke-31",
        occurredAt: createRelativeStoryIso(-2_400),
        status: "running",
        livePhase: "responding",
        upstreamAccountName: "watch-alpha@example.com",
        reasoningEffort: "medium",
        firstTokenMs: 860,
        tTotalMs: null,
      }),
      createPreview({
        id: 30,
        invokeId: "invoke-30",
        occurredAt: createRelativeStoryIso(-(11 * 60_000 + 39_000)),
        status: "completed",
        upstreamAccountName: "watch-alpha@example.com",
        model: "gpt-5.4-mini",
      }),
    ]),
  ]);
}

function createWarningSuccessUpstreamAccountActivityResponse(): UpstreamAccountActivityResponse {
  const response = createUpstreamAccountActivityStoryResponse();
  const firstAccount = response.accounts[0];
  if (!firstAccount) return response;

  const [firstRecent, ...restRecent] = firstAccount.recentInvocations;
  if (!firstRecent) return response;

  return {
    ...response,
    accounts: [
      {
        ...firstAccount,
        recentInvocations: [
          {
            ...firstRecent,
            status: "warning_success",
            failureKind: "downstream_closed",
            failureClass: "none",
            errorMessage: "[downstream_closed] downstream closed while streaming upstream response",
            totalTokens: 167_710,
            cost: 0.0629,
            tUpstreamTtfbMs: 1_131,
            tUpstreamStreamMs: 15_849,
            tTotalMs: 16_980,
          },
          ...restRecent,
        ],
      },
    ],
  };
}

function createUpstreamAccountRecentLayoutStoryResponse(): UpstreamAccountActivityResponse {
  const response = createUpstreamAccountActivityStoryResponse();
  const primaryAccount = response.accounts[0];
  if (!primaryAccount) return response;

  const errorAccount = {
    ...primaryAccount,
    recentInvocations: primaryAccount.recentInvocations.map((invocation, index) =>
      index === 0
        ? {
            ...invocation,
            status: "http_502" as const,
            failureClass: "service_failure" as const,
            failureKind: "upstream_http_5xx" as const,
            errorMessage: LONG_ERROR_SUMMARY,
            tUpstreamTtfbMs: null,
            tUpstreamStreamMs: null,
            tTotalMs: 21_006,
          }
        : invocation,
    ),
  };
  const normalAccount = {
    ...primaryAccount,
    upstreamAccountId: 87,
    displayName: "Pool Beta",
    groupName: "Fallback",
    lastError: null,
    recentInvocations: primaryAccount.recentInvocations.map((invocation, index) => ({
      ...invocation,
      id: invocation.id + 100,
      invokeId: `pool-beta-${invocation.invokeId}`,
      promptCacheKey: `pool-beta-${invocation.promptCacheKey ?? index}`,
      upstreamAccountId: 87,
      upstreamAccountName: "Pool Beta",
      status: "success" as const,
      failureClass: "none" as const,
      failureKind: "none" as const,
      errorMessage: undefined,
    })),
  };

  return {
    ...response,
    accounts: [errorAccount, normalAccount],
  };
}

function createRequestingOnlyResponse() {
  return createResponse([
    createConversation("pck-requesting-only", [
      createPreview({
        id: 32,
        invokeId: "invoke-32",
        occurredAt: createRelativeStoryIso(-750),
        status: "running",
        livePhase: "requesting",
        upstreamAccountName: "request-alpha@example.com",
        reasoningEffort: "medium",
        tUpstreamTtfbMs: null,
        firstTokenMs: null,
        tUpstreamStreamMs: null,
        tTotalMs: null,
      }),
      createPreview({
        id: 29,
        invokeId: "invoke-29",
        occurredAt: createRelativeStoryIso(-(12 * 60_000 + 18_000)),
        status: "completed",
        upstreamAccountName: "request-alpha@example.com",
        model: "gpt-5.4-mini",
      }),
    ]),
  ]);
}

function createPoolRoutingAccountStatesResponse() {
  return createResponse([
    createConversation("pck-routing-account-named", [
      createPreview({
        id: 41,
        invokeId: "invoke-routing-account-named",
        occurredAt: createRelativeStoryIso(-1_600),
        status: "running",
        livePhase: "responding",
        upstreamAccountId: 42,
        upstreamAccountName: "pool-alpha@example.com",
        firstTokenMs: 860,
        tTotalMs: null,
      }),
    ]),
    createConversation("pck-routing-account-missing", [
      createPreview({
        id: 42,
        invokeId: "invoke-routing-account-missing",
        occurredAt: createRelativeStoryIso(-3_200),
        status: "pending",
        livePhase: "requesting",
        upstreamAccountId: null,
        upstreamAccountName: null,
        tUpstreamTtfbMs: null,
        firstTokenMs: null,
        tUpstreamStreamMs: null,
        tTotalMs: null,
      }),
    ]),
    createConversation("pck-routing-account-terminal", [
      createPreview({
        id: 43,
        invokeId: "invoke-routing-account-terminal",
        occurredAt: createRelativeStoryIso(-8_000),
        status: "completed",
        upstreamAccountId: 42,
        upstreamAccountName: "pool-alpha@example.com",
      }),
    ]),
  ]);
}

const accountPlanBadgeResponse = createResponse([
  createConversation("pck-plan-enterprise", [
    createPreview({
      id: 221,
      invokeId: "invoke-plan-enterprise",
      occurredAt: "2026-04-04T10:04:58Z",
      status: "running",
      upstreamAccountName: "maximiliano.joseph8832.enterprise-routing-lab@example.com",
      upstreamAccountPlanType: "enterprise",
      reasoningEffort: "high",
      tTotalMs: null,
    }),
    createPreview({
      id: 220,
      invokeId: "invoke-plan-team",
      occurredAt: "2026-04-04T10:02:40Z",
      status: "completed",
      upstreamAccountName: "maximiliano.joseph8832.enterprise-routing-lab@example.com",
      upstreamAccountPlanType: "team",
      model: "gpt-5.4-mini",
    }),
  ]),
  createConversation("pck-plan-plus-free", [
    createPreview({
      id: 219,
      invokeId: "invoke-plan-plus",
      occurredAt: "2026-04-04T10:03:58Z",
      status: "completed",
      upstreamAccountName: "plus-account-osaka@example.com",
      upstreamAccountPlanType: "plus",
      reasoningEffort: "medium",
    }),
    createPreview({
      id: 218,
      invokeId: "invoke-plan-free",
      occurredAt: "2026-04-04T10:01:20Z",
      status: "completed",
      upstreamAccountName: "free-account-berlin@example.com",
      upstreamAccountPlanType: "free",
      model: "gpt-5.4-mini",
    }),
  ]),
]);

const transportBadgeResponse = createResponse([
  createConversation("pck-websocket-mixed", [
    createPreview({
      id: 36,
      invokeId: "invoke-ws-current",
      occurredAt: "2026-04-04T10:04:55Z",
      status: "running",
      transport: "websocket",
      upstreamAccountName: "ws-alpha@example.com",
      reasoningEffort: "medium",
      tTotalMs: null,
    }),
    createPreview({
      id: 35,
      invokeId: "invoke-http-previous",
      occurredAt: "2026-04-04T10:02:28Z",
      status: "completed",
      transport: null,
      upstreamAccountName: "ws-alpha@example.com",
      model: "gpt-5.4-mini",
    }),
  ]),
  createConversation("pck-http-control", [
    createPreview({
      id: 34,
      invokeId: "invoke-http-control",
      occurredAt: "2026-04-04T10:03:42Z",
      status: "completed",
      upstreamAccountName: "http-control@example.com",
      model: "gpt-5.4",
    }),
  ]),
]);

const failedClickableResponse = createResponse([
  createConversation("pck-failed-clickable", [
    createPreview({
      id: 41,
      invokeId: "invoke-41",
      occurredAt: "2026-04-04T10:03:40Z",
      status: "http_502",
      failureClass: "service_failure",
      errorMessage: "upstream gateway closed before first byte",
      failureKind: "upstream_timeout",
      reasoningEffort: "medium",
      upstreamAccountId: 77,
      upstreamAccountName: "pool-account-77@example.com",
      endpoint: "/v1/chat/completions",
      requestedServiceTier: "auto",
      serviceTier: "auto",
      responseContentEncoding: "identity",
      tUpstreamTtfbMs: null,
      tUpstreamStreamMs: null,
      tTotalMs: 30018,
    }),
    createPreview({
      id: 40,
      invokeId: "invoke-40",
      occurredAt: "2026-04-04T10:02:10Z",
      status: "completed",
      upstreamAccountId: 77,
      upstreamAccountName: "pool-account-77@example.com",
      model: "gpt-5.4-mini",
    }),
  ]),
]);

const failedStatusDedupResponse = createResponse([
  createConversation("pck-failed-status-dedup", [
    createPreview({
      id: 43,
      invokeId: "invoke-failed-dedup",
      occurredAt: "2026-04-04T10:03:58Z",
      status: "http_502",
      failureClass: "service_failure",
      errorMessage: "upstream gateway closed before first byte",
      failureKind: "upstream_timeout",
      reasoningEffort: "medium",
      upstreamAccountName: "pool-account-77@example.com",
      endpoint: "/v1/responses",
      tReqReadMs: 12,
      tReqParseMs: 8,
      tUpstreamConnectMs: 103,
      tUpstreamTtfbMs: 1_640,
      tUpstreamStreamMs: 0,
      tTotalMs: 13_050,
    }),
  ]),
]);

export {
  ADAPTIVE_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
  accountPlanBadgeResponse,
  BASE_UPSTREAM_ACCOUNT_RECENT_INVOCATION_OFFSETS_MS,
  BASE_UPSTREAM_ACCOUNT_RECENT_INVOCATION_SEEDS,
  BASE_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
  buildRecordFromPreview,
  buildUpstreamAccountRecentInvocations,
  contrastRatio,
  createConversation,
  createImageEditEndpointUpstreamAccountActivityStoryResponse,
  createPoolRoutingAccountStatesResponse,
  createPreview,
  createRelativeStoryIso,
  createRequestingOnlyResponse,
  createResponse,
  createRunningOnlyResponse,
  createUpstreamAccountActivityStoryResponse,
  createUpstreamAccountAdaptiveMetricsStoryResponse,
  createUpstreamAccountRecentLayoutStoryResponse,
  createWarningSuccessUpstreamAccountActivityResponse,
  currentAndPreviousResponse,
  currentOnlyResponse,
  DASHBOARD_STORY_PROMPT_CACHE_BINDING_ACCOUNTS,
  DashboardAccountWindowEvidenceSurface,
  enableConversationSelectionMode,
  ForcedWorkspaceViewStory,
  failedClickableResponse,
  failedStatusDedupResponse,
  imageEditInkChroma,
  imageEndpointChipResponse,
  isInFlightStatus,
  jsonResponse,
  LONG_ERROR_SUMMARY,
  oklabRelativeLuminance,
  openBulkClearBindingDialog,
  parseComputedColorChannels,
  perMinuteRate,
  readPerceptualChannel,
  relativeLuminance,
  requireFixture,
  StorySurface,
  selectConversationForBulkActions,
  transportBadgeResponse,
  UPSTREAM_ACCOUNT_ACTIVITY_WINDOW_MINUTES,
  useStoryTheme,
  warningSuccessConversationResponse,
};
