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

let host: HTMLDivElement | null = null;
let root: Root | null = null;

function renderTimeline(focusedAttemptId: number | null = null) {
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
    act(() => {
      root?.unmount();
    });
    host?.remove();
    root = null;
    host = null;
  });

  it("keeps the primary row focused on upstream evidence and reveals complete failure context on demand", async () => {
    fetchAttemptsMock.mockResolvedValue({
      items: [
        {
          id: 1,
          invokeId: "POOLCALL001",
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

    const table = host?.querySelector<HTMLTableElement>(
      '[data-testid="upstream-account-call-records-table"]',
    );
    expect(table).not.toBeNull();
    expect(table?.textContent).toMatch(/上游 HTTP 500|upstream http 500/i);
    expect(table?.textContent).toContain("JP Edge 01");
    expect(table?.querySelector("th")?.parentElement?.textContent).not.toMatch(
      /端点|endpoint/i,
    );
    const mobileTable = host?.querySelector<HTMLTableElement>(
      '[data-testid="upstream-account-call-records-mobile-table"]',
    );
    expect(mobileTable?.querySelector("a")?.className).toContain(
      "whitespace-nowrap",
    );
    expect(mobileTable?.querySelector("a")?.className).not.toContain(
      "truncate",
    );

    const disclosure = host?.querySelector<HTMLDetailsElement>(
      '[data-testid="account-attempt-evidence-1"]',
    );
    expect(disclosure).not.toBeNull();
    expect(disclosure?.closest("td")?.colSpan).toBe(7);
    expect(disclosure?.querySelector("summary")?.className).toContain(
      "min-h-11",
    );
    act(() => {
      disclosure?.querySelector("summary")?.dispatchEvent(
        new MouseEvent("click", { bubbles: true }),
      );
    });
    expect(disclosure?.open).toBe(true);
    expect(disclosure?.textContent).toContain("req_upstream_123");
    expect(disclosure?.textContent).toContain("route-tokyo-primary");
    expect(disclosure?.textContent).toMatch(/下游 HTTP|downstream http/i);
    expect(disclosure?.textContent).toContain("502");
    expect(disclosure?.textContent).toMatch(
      /upstream returned an oversized diagnostic payload/i,
    );
  });

  it("shows the pending attempt phase without adding another permanent column", async () => {
    fetchAttemptsMock.mockResolvedValue({
      items: [
        {
          id: 2,
          invokeId: "POOLPENDING",
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

    const table = host?.querySelector<HTMLTableElement>(
      '[data-testid="upstream-account-call-records-table"]',
    );
    expect(table?.textContent).toMatch(/等待首字节|waiting for first byte/i);
    expect(table?.querySelector("th")?.parentElement?.textContent).not.toMatch(
      /阶段|phase/i,
    );
  });

  it("opens the exact attempt diagnostics when event navigation focuses it", async () => {
    const focusedAttempt = {
      id: 3,
      invokeId: "POOLFOCUSED",
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

    renderTimeline();
    await flushAsync();
    renderTimeline(3);
    await flushAsync();

    const disclosures = host?.querySelectorAll<HTMLDetailsElement>(
      '[data-testid="account-attempt-evidence-3"]',
    );
    expect(disclosures).toHaveLength(2);
    expect(Array.from(disclosures ?? []).every((disclosure) => disclosure.open)).toBe(true);
  });
});
