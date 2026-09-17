/** @vitest-environment jsdom */

import { act } from "react";
import { expect, it, vi } from "vitest";
import type { ApiInvocation } from "../../lib/api";
import {
  apiMocks,
  createInvocationRecord,
  createWorkflowDetailFixture,
  renderInteractiveTable,
  renderTable,
  waitForCondition,
} from "./InvocationTable.test-support";
import { formatSecondsFromMilliseconds } from "./invocation-details-shared";

it("keeps invalid timing neutral and excludes it from summary averages", async () => {
  await renderInteractiveTable([
    {
      id: -94,
      invokeId: "invocation-invalid-timing",
      occurredAt: "2026-03-16T09:10:31Z",
      createdAt: "2026-03-16T09:10:31Z",
      source: "proxy",
      proxyDisplayName: "relay-invalid",
      endpoint: "/v1/responses",
      model: "gpt-5.4",
      status: "success",
      firstTokenMs: -100,
      tUpstreamStreamMs: -100,
    },
    {
      id: -95,
      invokeId: "invocation-measured-timing",
      occurredAt: "2026-03-16T09:10:30Z",
      createdAt: "2026-03-16T09:10:30Z",
      source: "proxy",
      proxyDisplayName: "relay-measured",
      endpoint: "/v1/responses",
      model: "gpt-5.4",
      status: "success",
      firstTokenMs: 700,
      tUpstreamStreamMs: 1_000,
    },
  ]);

  const invalidFirstToken = document.querySelector('[data-testid="invocation-card-ttft"]');
  expect(invalidFirstToken?.textContent).toContain("--");
  expect(invalidFirstToken?.className).not.toContain("text-success");
  const summary = document.querySelector('[data-testid="invocation-card-summary-ttft"]');
  expect(summary?.textContent).toContain("0.7 s");
  expect(summary?.textContent).toContain("1 s");
  expect(summary?.textContent).not.toContain("0.3 s");
});
it("treats zero response duration as unavailable while preserving zero TTFT", async () => {
  await renderInteractiveTable([
    {
      id: -96,
      invokeId: "invocation-zero-response-duration",
      occurredAt: "2026-03-16T09:10:32Z",
      createdAt: "2026-03-16T09:10:32Z",
      source: "proxy",
      proxyDisplayName: "relay-zero-response",
      endpoint: "/v1/responses",
      model: "gpt-5.4",
      status: "success",
      firstTokenMs: 0,
      tUpstreamStreamMs: 0,
    },
  ]);

  expect(document.querySelector('[data-testid="invocation-card-ttft"]')?.textContent).toContain(
    "0 s",
  );
  expect(document.querySelector('[data-testid="invocation-card-ttft"]')?.className).toContain(
    "text-success",
  );
  expect(document.querySelector('[data-testid="invocation-card-response"]')?.textContent).toContain(
    "--",
  );
  expect(
    document.querySelector('[data-testid="invocation-card-summary-ttft"]')?.textContent,
  ).toContain("—");
});
it("keeps nonfinite timing unavailable in cards and summaries", async () => {
  await renderInteractiveTable([
    {
      id: -97,
      invokeId: "invocation-infinite-timing",
      occurredAt: "2026-03-16T09:10:33Z",
      createdAt: "2026-03-16T09:10:33Z",
      source: "proxy",
      proxyDisplayName: "relay-infinite",
      endpoint: "/v1/responses",
      model: "gpt-5.4",
      status: "success",
      firstTokenMs: Number.POSITIVE_INFINITY,
      tUpstreamStreamMs: Number.NaN,
    },
  ]);

  expect(document.querySelector('[data-testid="invocation-card-ttft"]')?.textContent).toContain(
    "--",
  );
  expect(document.querySelector('[data-testid="invocation-card-response"]')?.textContent).toContain(
    "--",
  );
  expect(
    document.querySelector('[data-testid="invocation-card-summary-ttft"]')?.textContent,
  ).toContain("—");
  expect(
    document.querySelector('[data-testid="invocation-card-summary-ttft"]')?.textContent,
  ).toContain("—");
});
it("keeps negative TTFT unavailable in the expanded invocation view model", () => {
  expect(formatSecondsFromMilliseconds(-1_500, "zh-CN")).toBe("—");
  expect(formatSecondsFromMilliseconds(1_234, "zh-CN")).toBe("1.2 s");
  expect(formatSecondsFromMilliseconds(99_950, "zh-CN")).toBe("100 s");
});
it("keeps TTFT measured while a responding invocation has no completed response duration", async () => {
  await renderInteractiveTable([
    {
      id: -92,
      invokeId: "invocation-running-response-duration",
      occurredAt: "2026-03-16T09:10:30Z",
      createdAt: "2026-03-16T09:10:30Z",
      source: "proxy",
      proxyDisplayName: "relay-running",
      endpoint: "/v1/responses",
      model: "gpt-5.4",
      status: "running",
      firstTokenMs: 742.6,
      tUpstreamStreamMs: null,
    },
  ]);

  expect(document.querySelector('[data-testid="invocation-phase-badge"]')?.textContent).toContain(
    "响应中",
  );
  expect(document.querySelector('[data-testid="invocation-card-ttft"]')?.textContent).toContain(
    "0.7 s",
  );
  expect(document.querySelector('[data-testid="invocation-card-response"]')?.textContent).toContain(
    "--",
  );
});
it("shows compact latency without decimals after it rounds to 100 seconds", async () => {
  await renderInteractiveTable([
    {
      id: -93,
      invokeId: "invocation-latency-rounding-boundary",
      occurredAt: "2026-03-16T09:10:30Z",
      createdAt: "2026-03-16T09:10:30Z",
      source: "proxy",
      proxyDisplayName: "relay-latency-boundary",
      endpoint: "/v1/responses",
      model: "gpt-5.4",
      status: "success",
      firstTokenMs: 99_950,
      tUpstreamStreamMs: 100_040,
    },
  ]);

  expect(document.querySelector('[data-testid="invocation-card-ttft"]')?.textContent).toContain(
    "100 s",
  );
  expect(document.querySelector('[data-testid="invocation-card-response"]')?.textContent).toContain(
    "100 s",
  );
  expect(document.body.textContent).not.toContain("100.0 s");
});
it("forwards pool account clicks to the shared upstream account controller", async () => {
  const onOpenUpstreamAccount = vi.fn();

  await renderInteractiveTable(
    [
      {
        id: 41,
        invokeId: "pool-drawer-open",
        occurredAt: "2026-03-16T09:10:30Z",
        createdAt: "2026-03-16T09:10:30Z",
        source: "proxy",
        routeMode: "pool",
        upstreamAccountId: 42,
        upstreamAccountName: "Pool Alpha",
        proxyDisplayName: "relay-alpha",
        responseContentEncoding: "gzip",
        endpoint: "/v1/responses",
        model: "gpt-5.4",
        status: "success",
        totalTokens: 512,
        tUpstreamTtfbMs: 104.4,
        tTotalMs: 702.3,
      },
    ],
    { onOpenUpstreamAccount },
  );

  const trigger = Array.from(document.querySelectorAll("button")).find((button) =>
    button.textContent?.includes("Pool Alpha"),
  );
  expect(trigger).toBeTruthy();

  await act(async () => {
    trigger?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await Promise.resolve();
  });

  expect(onOpenUpstreamAccount).toHaveBeenCalledWith(42, "Pool Alpha");
  expect(
    document.querySelector('[data-testid="invocation-card"]')?.getAttribute("data-expanded"),
  ).toBe("false");
});
it("toggles the existing detail panel from card click and keyboard activation", async () => {
  const record = createInvocationRecord(0);
  apiMocks.fetchInvocationWorkflowDetail.mockResolvedValueOnce(createWorkflowDetailFixture(record));
  await renderInteractiveTable([record]);

  const card = document.querySelector('[data-testid="invocation-card"]') as HTMLElement;
  expect(card.getAttribute("data-expanded")).toBe("false");

  await act(async () => {
    card.click();
    await Promise.resolve();
  });
  await waitForCondition(() => card.getAttribute("data-expanded") === "true");
  expect(document.body.textContent).toContain("工作流时间线");

  await act(async () => {
    card.focus();
    card.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await Promise.resolve();
  });
  expect(card.getAttribute("data-expanded")).toBe("false");

  const chevron = Array.from(document.querySelectorAll("button")).find(
    (button) =>
      button.getAttribute("aria-label") === "展开详情" ||
      button.getAttribute("aria-label") === "Show details",
  ) as HTMLButtonElement | undefined;
  expect(chevron).toBeTruthy();
  await act(async () => {
    chevron?.focus();
    chevron?.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    chevron?.click();
    await Promise.resolve();
  });
  expect(card.getAttribute("data-expanded")).toBe("true");
});
it("does not render a local account detail drawer after clicking a pool account name", async () => {
  await renderInteractiveTable([
    {
      id: 41,
      invokeId: "pool-no-local-drawer",
      occurredAt: "2026-03-16T09:10:30Z",
      createdAt: "2026-03-16T09:10:30Z",
      source: "proxy",
      routeMode: "pool",
      upstreamAccountId: 42,
      upstreamAccountName: "Pool Alpha",
      proxyDisplayName: "relay-alpha",
      responseContentEncoding: "gzip",
      endpoint: "/v1/responses",
      model: "gpt-5.4",
      status: "success",
    },
  ]);

  const trigger = Array.from(document.querySelectorAll("button")).find((button) =>
    button.textContent?.includes("Pool Alpha"),
  );
  expect(trigger).toBeTruthy();

  await act(async () => {
    trigger?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await Promise.resolve();
  });

  expect(document.body.querySelector('[role="dialog"]')).toBeNull();
  expect(document.body.textContent).not.toContain("去号池查看完整详情");
});
it("keeps structured-only metadata out of summary rows", () => {
  const records: ApiInvocation[] = [
    {
      id: 4,
      invokeId: "invocation-detail-pruned",
      occurredAt: "2026-03-07T03:13:52Z",
      createdAt: "2026-03-07T03:13:52Z",
      source: "proxy",
      proxyDisplayName: "hkg-edge-4",
      endpoint: "/v1/responses",
      model: "gpt-5.4",
      status: "success",
      inputTokens: 1024,
      outputTokens: 64,
      totalTokens: 1088,
      cost: 0.0021,
      detailLevel: "structured_only",
      detailPrunedAt: "2026-02-01T12:34:56Z",
      detailPruneReason: "success_over_30d",
    },
  ];

  const summaryHtml = renderTable(records);
  expect(summaryHtml).not.toContain('data-testid="invocation-detail-level-badge"');
  expect(summaryHtml).not.toContain("Structured only");
  expect(summaryHtml).not.toContain("精简于 2026-02-01 12:34:56Z");
});
it("keeps legacy full-detail records out of summary rows", () => {
  const records: ApiInvocation[] = [
    {
      id: 5,
      invokeId: "invocation-detail-full-default",
      occurredAt: "2026-03-07T03:13:50Z",
      createdAt: "2026-03-07T03:13:50Z",
      source: "xy",
      endpoint: "/v1/chat/completions",
      model: "gpt-4.1",
      status: "failed",
      errorMessage: "legacy row still renders",
    },
  ];

  const summaryHtml = renderTable(records);
  expect(summaryHtml).not.toContain('data-testid="invocation-detail-level-badge"');
  expect(summaryHtml).not.toContain("Full");
  expect(summaryHtml).not.toContain("Structured only");
  expect(summaryHtml).not.toContain("精简于");
  expect(summaryHtml).toContain("legacy row still renders");
});
