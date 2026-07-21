/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "../../i18n";
import {
  fetchForwardProxyBindingNodes,
  fetchUpstreamAccountAttempts,
  locateUpstreamAccountAttempt,
} from "../../lib/api";
import { UpstreamAccountAttemptTimeline } from "./UpstreamAccountAttemptTimeline";

vi.mock("../../lib/api", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../lib/api")>()),
  fetchForwardProxyBindingNodes: vi.fn(),
  fetchUpstreamAccountAttempts: vi.fn(),
  locateUpstreamAccountAttempt: vi.fn(),
}));

const fetchAttemptsMock = vi.mocked(fetchUpstreamAccountAttempts);
const fetchBindingNodesMock = vi.mocked(fetchForwardProxyBindingNodes);
const originalScrollIntoView = HTMLElement.prototype.scrollIntoView;

let host: HTMLDivElement | null = null;
let root: Root | null = null;
let interactionBoundary: HTMLDivElement | null = null;
let scrollIntoViewMock = vi.fn();

function renderTimeline({
  focusedAttemptId = null,
  focusVersion = 0,
  onFocusRequestHandled,
  boundary = null,
}: {
  focusedAttemptId?: string | null;
  focusVersion?: number;
  onFocusRequestHandled?: (version: number) => void;
  boundary?: HTMLElement | null;
} = {}) {
  if (!host) {
    host = document.createElement("div");
    document.body.appendChild(host);
  }
  if (!root) root = createRoot(host);
  act(() => {
    root?.render(
      <MemoryRouter>
        <I18nProvider>
          <UpstreamAccountAttemptTimeline
            accountId={101}
            focusedAttemptId={focusedAttemptId}
            focusVersion={focusVersion}
            interactionBoundary={boundary}
            onFocusRequestHandled={onFocusRequestHandled}
          />
        </I18nProvider>
      </MemoryRouter>,
    );
  });
}

async function flushAsync() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
  });
}

describe("UpstreamAccountAttemptTimeline", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useRealTimers();
    scrollIntoViewMock = vi.fn();
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
      configurable: true,
      value: scrollIntoViewMock,
    });
    vi.mocked(locateUpstreamAccountAttempt).mockResolvedValue({
      items: [],
      total: 0,
      page: 1,
      pageSize: 50,
    });
    fetchBindingNodesMock.mockResolvedValue([
      {
        key: "jp-edge-01",
        source: "manual",
        displayName: "JP Edge 01",
        protocolLabel: "HTTP",
        egressIp: null,
        egressIpCheckedAt: null,
        egressIpProvider: null,
        egressIpError: null,
        egressIpErrorAt: null,
        penalized: false,
        selectable: true,
        last24h: [],
      },
    ]);
  });

  afterEach(() => {
    vi.useRealTimers();
    act(() => {
      root?.unmount();
    });
    host?.remove();
    interactionBoundary?.remove();
    interactionBoundary = null;
    root = null;
    host = null;
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
      configurable: true,
      value: originalScrollIntoView,
    });
  });

  it("keeps the primary row focused on upstream evidence and reveals complete failure context on demand", async () => {
    fetchAttemptsMock.mockResolvedValue({
      items: [
        {
          attemptId: "4V7MYPJG",
          invokeId: "K7QM9ZD4HP",
          occurredAt: "2026-07-11T12:00:00.000Z",
          endpoint: "/v1/responses",
          upstreamAccountId: 101,
          requestModel: "gpt-5.4",
          responseModel: "gpt-5.4-2026-07-01",
          proxyBindingKeySnapshot: "jp-edge-01",
          attemptIndex: 1,
          distinctAccountIndex: 0,
          sameAccountRetryIndex: 0,
          status: "http_failure",
          phase: "failed",
          httpStatus: 500,
          downstreamHttpStatus: 502,
          failureKind: "upstream_response_failed",
          errorMessage: "upstream returned an oversized diagnostic payload",
          connectLatencyMs: 120,
          firstByteLatencyMs: 480,
          streamLatencyMs: 810,
          downstreamRequestContentEncoding: "gzip",
          upstreamRequestCompressionAlgorithm: "zstd",
          upstreamRequestCompressionMode: "recompressed",
          logicalBodyBytes: 1000,
          transmittedBodyBytes: 580,
          savedBytes: 420,
          ratioPct: -42,
          approxUploadBytes: 644,
          approxDownloadBytes: 812,
          upstreamRequestId: "req_upstream_123",
          upstreamRouteKey: "route-tokyo-primary",
          createdAt: "2026-07-11T12:00:00.000Z",
        },
      ],
      total: 1,
      page: 1,
      pageSize: 50,
    });

    renderTimeline();
    await flushAsync();

    const list = host?.querySelector<HTMLElement>('[data-testid="upstream-account-attempt-list"]');
    expect(list).not.toBeNull();
    const card = host?.querySelector<HTMLElement>(
      '[data-testid="account-attempt-record-4V7MYPJG"]',
    );
    expect(card).not.toBeNull();
    expect(card?.textContent).toMatch(/上游 HTTP 500|upstream http 500/i);
    expect(card?.textContent).toContain("K7QM9ZD4HP");
    expect(card?.textContent).not.toContain("POOLCALL001");
    const requestHeadersButton = Array.from(card?.querySelectorAll("button") ?? []).find((button) =>
      /请求头|headers/i.test(button.textContent ?? ""),
    );
    expect(requestHeadersButton).toBeDefined();
    act(() => {
      requestHeadersButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(card?.textContent).toContain("JP Edge 01");
    expect(card?.textContent).toContain("route-tokyo-primary");
    const requestBodyButton = Array.from(card?.querySelectorAll("button") ?? []).find((button) =>
      /请求体|body/i.test(button.textContent ?? ""),
    );
    expect(requestBodyButton).toBeDefined();
    act(() => {
      requestBodyButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(card?.textContent).toMatch(/http request compression|http 请求压缩/i);
    expect(card?.textContent).toMatch(/zstd/i);
    expect(card?.textContent).toContain("-42% (1,000 B -> 580 B)");
    const timingButton = Array.from(card?.querySelectorAll("button") ?? []).find((button) =>
      /时间|timing/i.test(button.textContent ?? ""),
    );
    expect(timingButton).toBeDefined();
    act(() => {
      timingButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(card?.textContent).toContain("req_upstream_123");
    const responseButton = Array.from(card?.querySelectorAll("button") ?? []).find((button) =>
      /^(响应|response)/i.test((button.textContent ?? "").trim()),
    );
    expect(responseButton).toBeDefined();
    act(() => {
      responseButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(card?.textContent).toMatch(/upstream returned an oversized diagnostic payload/i);
  });

  it("shows the pending attempt phase without adding another permanent column", async () => {
    fetchAttemptsMock.mockResolvedValue({
      items: [
        {
          attemptId: "QADKN5Z9",
          invokeId: "M8R7XZ4Q2W",
          occurredAt: "2026-07-11T12:00:00.000Z",
          endpoint: "/v1/responses",
          upstreamAccountId: 101,
          requestModel: "gpt-5.4",
          proxyBindingKeySnapshot: "__direct__",
          attemptIndex: 1,
          distinctAccountIndex: 0,
          sameAccountRetryIndex: 0,
          status: "pending",
          phase: "waiting_first_byte",
          connectLatencyMs: 80,
          createdAt: "2026-07-11T12:00:00.000Z",
        },
      ],
      total: 1,
      page: 1,
      pageSize: 50,
    });

    renderTimeline();
    await flushAsync();

    const card = host?.querySelector<HTMLElement>(
      '[data-testid="account-attempt-record-QADKN5Z9"]',
    );
    expect(card).not.toBeNull();
    expect(card?.textContent).toContain("waiting_first_byte");
    expect(card?.textContent).not.toMatch(/阶段|phase/i);
  });

  it("scrolls, highlights, and fades the focused attempt after the next drawer interaction", async () => {
    vi.useFakeTimers();
    const focusedAttempt = {
      attemptId: "YG7P25XG",
      invokeId: "YG7P25XG9K",
      occurredAt: "2026-07-11T12:00:00.000Z",
      endpoint: "/v1/responses",
      upstreamAccountId: 101,
      requestModel: "gpt-5.4",
      proxyBindingKeySnapshot: "jp-edge-01",
      attemptIndex: 1,
      distinctAccountIndex: 0,
      sameAccountRetryIndex: 0,
      status: "http_failure",
      phase: "failed",
      httpStatus: 500,
      errorMessage: "focused failure details",
      createdAt: "2026-07-11T12:00:00.000Z",
    };
    fetchAttemptsMock.mockResolvedValue({
      items: [focusedAttempt],
      total: 1,
      page: 1,
      pageSize: 50,
    });
    vi.mocked(locateUpstreamAccountAttempt).mockResolvedValue({
      items: [focusedAttempt],
      total: 1,
      page: 1,
      pageSize: 50,
    });
    const onFocusRequestHandled = vi.fn();
    interactionBoundary = document.createElement("div");
    document.body.appendChild(interactionBoundary);

    renderTimeline();
    await flushAsync();
    renderTimeline({
      focusedAttemptId: "YG7P25XG",
      focusVersion: 1,
      boundary: interactionBoundary,
      onFocusRequestHandled,
    });
    await flushAsync();

    const record = host?.querySelector<HTMLElement>(
      '[data-testid="account-attempt-record-YG7P25XG"]',
    );
    expect(record).not.toBeNull();
    expect(locateUpstreamAccountAttempt).toHaveBeenCalledWith(
      101,
      "YG7P25XG",
      expect.objectContaining({
        pageSize: 50,
        signal: expect.any(AbortSignal),
      }),
    );
    expect(onFocusRequestHandled).toHaveBeenCalledWith(1);
    expect(scrollIntoViewMock).toHaveBeenCalledWith({
      behavior: "smooth",
      block: "nearest",
    });
    expect(record?.dataset.focusVisible).toBe("true");
    expect(record?.getAttribute("aria-current")).toBe("true");
    expect(record?.textContent).toMatch(/关键诊断|key diagnostics/i);
    expect(record?.textContent).toMatch(/上游 HTTP 状态|upstream http/i);

    act(() => {
      interactionBoundary?.dispatchEvent(new Event("pointerdown", { bubbles: true }));
    });
    expect(record?.dataset.focusVisible).toBe("true");

    act(() => {
      vi.advanceTimersByTime(1_499);
    });
    expect(record?.dataset.focusVisible).toBe("true");

    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(record?.dataset.focusVisible).toBe("false");

    renderTimeline({
      focusedAttemptId: "YG7P25XG",
      focusVersion: 2,
      boundary: interactionBoundary,
    });
    await flushAsync();
    const refocusedRecord = host?.querySelector<HTMLElement>(
      '[data-testid="account-attempt-record-YG7P25XG"]',
    );
    expect(refocusedRecord?.dataset.focusVisible).toBe("true");
    expect(scrollIntoViewMock).toHaveBeenCalledTimes(2);
  });

  it("shows locate unavailable feedback when the focused attempt is outside the locate window", async () => {
    vi.mocked(locateUpstreamAccountAttempt).mockRejectedValue(new Error("404 not found"));

    renderTimeline({
      focusedAttemptId: "MISS1234",
      focusVersion: 1,
    });
    await flushAsync();

    expect(host?.textContent).toMatch(/7-day retention window|7 天保留范围|7 天窗口/i);
  });
});
