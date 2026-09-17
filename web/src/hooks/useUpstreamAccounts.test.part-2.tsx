/** @vitest-environment jsdom */
import { act } from "react";
import { expect, it, vi } from "vitest";
import type { UpstreamAccountDetail, UpstreamAccountListResponse } from "../lib/api";
import { UPSTREAM_ACCOUNTS_OPEN_RESYNC_COOLDOWN_MS } from "./useUpstreamAccounts";
import {
  apiMocks,
  click,
  createDetail,
  createListResponse,
  createSummary,
  deferred,
  emitOpenEvent,
  emitRecordsEvent,
  flushAsync,
  Probe,
  render,
  text,
} from "./useUpstreamAccounts.test-support";

it("keeps account detail errors scoped per account", async () => {
  apiMocks.fetchUpstreamAccountDetail
    .mockRejectedValueOnce(new Error("Alpha failed"))
    .mockRejectedValueOnce(new Error("Beta failed"))
    .mockRejectedValueOnce(new Error("Alpha failed"));

  render(<Probe />);
  await flushAsync();

  expect(text("selected-id")).toBe("1");
  expect(text("detail-error")).toBe("Alpha failed");

  click("select-beta");
  await flushAsync();
  expect(text("selected-id")).toBe("2");
  expect(text("detail-error")).toBe("Beta failed");

  click("select-alpha");
  await flushAsync();

  expect(text("selected-id")).toBe("1");
  expect(text("detail-error")).toBe("Alpha failed");
});
it("does not clear list errors after a non-list success", async () => {
  apiMocks.fetchUpstreamAccountDetail.mockResolvedValue(createDetail(1, "Alpha"));
  apiMocks.fetchUpstreamAccounts
    .mockResolvedValueOnce(createListResponse())
    .mockRejectedValueOnce(new Error("List failed"));
  apiMocks.reloginUpstreamAccount.mockResolvedValueOnce({
    loginId: "relogin-1",
  });

  render(<Probe />);
  await flushAsync();

  click("refresh");
  await flushAsync();
  expect(text("list-error")).toBe("List failed");

  click("relogin-alpha");
  await flushAsync();
  expect(text("list-error")).toBe("List failed");
});
it("clears an account error after that account sync succeeds off-selection", async () => {
  const sync = deferred<UpstreamAccountDetail>();
  apiMocks.fetchUpstreamAccountDetail
    .mockRejectedValueOnce(new Error("Alpha failed"))
    .mockResolvedValueOnce(createDetail(2, "Beta"))
    .mockResolvedValueOnce(createDetail(2, "Beta"))
    .mockResolvedValueOnce(createDetail(2, "Beta"))
    .mockResolvedValueOnce(createDetail(1, "Alpha synced"));
  apiMocks.syncUpstreamAccount.mockImplementationOnce(async () => sync.promise);

  render(<Probe />);
  await flushAsync();

  expect(text("selected-id")).toBe("1");
  expect(text("detail-error")).toBe("Alpha failed");

  click("sync-alpha");
  click("select-beta");
  await flushAsync();

  sync.resolve(createDetail(1, "Alpha synced"));
  await flushAsync();

  click("select-alpha");
  await flushAsync();
  expect(apiMocks.fetchUpstreamAccountDetail).toHaveBeenNthCalledWith(
    5,
    1,
    expect.objectContaining({
      signal: expect.any(AbortSignal),
      includeRecentActions: undefined,
    }),
  );
  expect(text("selected-id")).toBe("1");
  expect(text("detail-error")).toBe("");
});
it("does not reclaim selection when a delete finishes after switching away", async () => {
  const remove = deferred<void>();
  apiMocks.fetchUpstreamAccountDetail
    .mockResolvedValueOnce(createDetail(1, "Alpha"))
    .mockResolvedValueOnce(createDetail(2, "Beta"));
  apiMocks.fetchUpstreamAccounts
    .mockResolvedValueOnce({
      writesEnabled: true,
      items: [createSummary(1, "Alpha"), createSummary(3, "Gamma"), createSummary(2, "Beta")],
      groups: [],
      hasUngroupedAccounts: false,
      routing: {
        writesEnabled: true,
        apiKeyConfigured: false,
        maskedApiKey: null,
      },
    })
    .mockResolvedValueOnce({
      writesEnabled: true,
      items: [createSummary(2, "Beta"), createSummary(3, "Gamma")],
      groups: [],
      hasUngroupedAccounts: false,
      routing: {
        writesEnabled: true,
        apiKeyConfigured: false,
        maskedApiKey: null,
      },
    });
  apiMocks.deleteUpstreamAccount.mockImplementationOnce(async () => remove.promise);

  render(<Probe />);
  await flushAsync();

  click("remove-alpha");
  click("select-beta");
  await flushAsync();

  remove.resolve();
  await flushAsync();

  expect(text("selected-id")).toBe("2");
  expect(text("selected-name")).toBe("Beta");
});
it("reanchors away from a deleted current account even if the list refresh fails", async () => {
  apiMocks.fetchUpstreamAccounts
    .mockResolvedValueOnce(createListResponse())
    .mockRejectedValueOnce(new Error("List failed"));
  apiMocks.fetchUpstreamAccountDetail
    .mockResolvedValueOnce(createDetail(1, "Alpha"))
    .mockResolvedValueOnce(createDetail(2, "Beta"));
  apiMocks.deleteUpstreamAccount.mockResolvedValueOnce();

  render(<Probe />);
  await flushAsync();

  expect(text("selected-id")).toBe("1");
  expect(text("detail-id")).toBe("1");

  click("remove-alpha");
  await flushAsync();
  await flushAsync();

  expect(text("selected-id")).toBe("2");
  expect(text("selected-name")).toBe("Beta");
  expect(text("detail-id")).not.toBe("1");
  expect(text("detail-name")).not.toBe("Alpha");
});
it("loads recent actions on demand and preserves them across a plain detail refresh", async () => {
  apiMocks.fetchUpstreamAccounts.mockResolvedValue(createListResponse());
  apiMocks.fetchUpstreamAccountDetail
    .mockResolvedValueOnce(createDetail(1, "Alpha"))
    .mockResolvedValueOnce({
      ...createDetail(1, "Alpha"),
      recentActions: [
        {
          id: 7,
          occurredAt: "2026-03-16T02:06:00.000Z",
          action: "route_hard_unavailable",
          source: "call",
          createdAt: "2026-03-16T02:06:00.000Z",
        },
      ],
    })
    .mockResolvedValueOnce(createDetail(1, "Alpha Updated"));

  render(<Probe />);
  await flushAsync();

  expect(apiMocks.fetchUpstreamAccountDetail).toHaveBeenNthCalledWith(1, 1, {
    signal: expect.any(AbortSignal),
    includeRecentActions: undefined,
  });
  expect(text("detail-recent-actions-count")).toBe("0");

  click("load-detail-recent-actions");
  await flushAsync();

  expect(apiMocks.fetchUpstreamAccountDetail).toHaveBeenNthCalledWith(2, 1, {
    signal: expect.any(AbortSignal),
    includeRecentActions: true,
  });
  expect(text("detail-recent-actions-count")).toBe("1");

  click("refresh");
  await flushAsync();

  expect(apiMocks.fetchUpstreamAccountDetail).toHaveBeenNthCalledWith(3, 1, {
    signal: expect.any(AbortSignal),
    includeRecentActions: undefined,
  });
  expect(text("detail-name")).toBe("Alpha Updated");
  expect(text("detail-recent-actions-count")).toBe("1");
});
it("invalidates an older detail reload before sync refreshes the list", async () => {
  const refreshedDetail = deferred<UpstreamAccountDetail>();
  const syncedList = deferred<UpstreamAccountListResponse>();
  const sync = deferred<UpstreamAccountDetail>();

  apiMocks.fetchUpstreamAccounts
    .mockResolvedValueOnce(createListResponse())
    .mockResolvedValueOnce(createListResponse())
    .mockImplementationOnce(async () => syncedList.promise);
  apiMocks.fetchUpstreamAccountDetail
    .mockResolvedValueOnce(createDetail(1, "Alpha"))
    .mockImplementationOnce(async () => refreshedDetail.promise)
    .mockResolvedValue(createDetail(1, "Alpha Synced"));
  apiMocks.syncUpstreamAccount.mockImplementationOnce(async () => sync.promise);

  render(<Probe />);
  await flushAsync();

  click("refresh");
  await flushAsync();
  click("sync-alpha");
  await flushAsync();

  sync.resolve(createDetail(1, "Alpha Synced"));
  await flushAsync();

  refreshedDetail.resolve(createDetail(1, "Alpha Stale"));
  await flushAsync();
  expect(text("detail-name")).toBe("Alpha");

  syncedList.resolve(createListResponse());
  await flushAsync();
  await flushAsync();
  expect(text("detail-name")).toBe("Alpha Synced");
});
it("refreshes the final selected account after switching during refresh", async () => {
  const refreshedList = deferred<UpstreamAccountListResponse>();
  apiMocks.fetchUpstreamAccounts
    .mockResolvedValueOnce(createListResponse())
    .mockImplementationOnce(async () => refreshedList.promise);
  apiMocks.fetchUpstreamAccountDetail
    .mockResolvedValueOnce(createDetail(1, "Alpha"))
    .mockResolvedValueOnce(createDetail(2, "Beta Stale"))
    .mockResolvedValueOnce(createDetail(2, "Beta Fresh"));

  render(<Probe />);
  await flushAsync();

  click("refresh");
  click("select-beta");
  await flushAsync();

  expect(text("selected-id")).toBe("2");
  expect(text("detail-name")).toBe("Beta Stale");

  refreshedList.resolve(createListResponse());
  await flushAsync();

  expect(apiMocks.fetchUpstreamAccountDetail).toHaveBeenNthCalledWith(
    3,
    2,
    expect.objectContaining({
      signal: expect.any(AbortSignal),
      includeRecentActions: undefined,
    }),
  );
  expect(text("selected-id")).toBe("2");
  expect(text("detail-id")).toBe("2");
  expect(text("detail-name")).toBe("Beta Fresh");
});
it("ignores an older list refresh after sync starts a newer list reload", async () => {
  const staleRefreshList = deferred<UpstreamAccountListResponse>();
  const syncedList = deferred<UpstreamAccountListResponse>();
  const sync = deferred<UpstreamAccountDetail>();

  apiMocks.fetchUpstreamAccounts
    .mockResolvedValueOnce(createListResponse())
    .mockImplementationOnce(async () => staleRefreshList.promise)
    .mockImplementationOnce(async () => syncedList.promise)
    .mockResolvedValue({
      writesEnabled: true,
      items: [createSummary(1, "Alpha Synced"), createSummary(2, "Beta")],
      groups: [],
      hasUngroupedAccounts: false,
      routing: {
        writesEnabled: true,
        apiKeyConfigured: false,
        maskedApiKey: null,
      },
    });
  apiMocks.fetchUpstreamAccountDetail
    .mockResolvedValueOnce(createDetail(1, "Alpha"))
    .mockResolvedValue(createDetail(1, "Alpha Synced"));
  apiMocks.syncUpstreamAccount.mockImplementationOnce(async () => sync.promise);

  render(<Probe />);
  await flushAsync();

  click("refresh");
  await flushAsync();
  click("sync-alpha");
  await flushAsync();

  sync.resolve(createDetail(1, "Alpha Synced"));
  await flushAsync();

  syncedList.resolve({
    writesEnabled: true,
    items: [createSummary(1, "Alpha Synced"), createSummary(2, "Beta")],
    groups: [],
    hasUngroupedAccounts: false,
    routing: {
      writesEnabled: true,
      apiKeyConfigured: false,
      maskedApiKey: null,
    },
  });
  await flushAsync();
  await flushAsync();

  expect(text("selected-name")).toBe("Alpha Synced");
  expect(text("detail-name")).toBe("Alpha Synced");

  staleRefreshList.resolve({
    writesEnabled: true,
    items: [createSummary(1, "Alpha Stale"), createSummary(2, "Beta")],
    groups: [],
    hasUngroupedAccounts: false,
    routing: {
      writesEnabled: true,
      apiKeyConfigured: false,
      maskedApiKey: null,
    },
  });
  await flushAsync();

  expect(text("selected-id")).toBe("1");
  expect(text("selected-name")).toBe("Alpha Synced");
  expect(text("detail-name")).toBe("Alpha Synced");
});
it("does not refresh the roster after records SSE events", async () => {
  apiMocks.fetchUpstreamAccountDetail.mockResolvedValue(createDetail(1, "Alpha"));
  render(<Probe />);
  await flushAsync();

  expect(apiMocks.fetchUpstreamAccounts).toHaveBeenCalledTimes(1);
  expect(apiMocks.fetchUpstreamAccountDetail).toHaveBeenCalledTimes(1);
  expect(apiMocks.fetchUpstreamAccountWindowUsage).not.toHaveBeenCalled();
  expect(text("detail-loading")).toBe("false");
  expect(text("window-usage-pending")).toBe("false");

  vi.useFakeTimers();
  try {
    act(() => {
      emitRecordsEvent();
      vi.advanceTimersByTime(10_000);
    });
    await flushAsync();

    expect(apiMocks.fetchUpstreamAccounts).toHaveBeenCalledTimes(1);
    expect(apiMocks.fetchUpstreamAccountDetail).toHaveBeenCalledTimes(1);
    expect(apiMocks.fetchUpstreamAccountWindowUsage).not.toHaveBeenCalled();
    expect(text("detail-loading")).toBe("false");
    expect(text("window-usage-pending")).toBe("false");
  } finally {
    vi.useRealTimers();
  }
});
it("skips an immediate open resync after a fresh load, then resyncs after the cooldown", async () => {
  apiMocks.fetchUpstreamAccountDetail.mockResolvedValue(createDetail(1, "Alpha"));

  render(<Probe />);
  await flushAsync();

  expect(apiMocks.fetchUpstreamAccounts).toHaveBeenCalledTimes(1);

  vi.useFakeTimers();
  try {
    act(() => {
      emitOpenEvent();
    });
    await flushAsync();
    expect(apiMocks.fetchUpstreamAccounts).toHaveBeenCalledTimes(1);

    act(() => {
      vi.advanceTimersByTime(UPSTREAM_ACCOUNTS_OPEN_RESYNC_COOLDOWN_MS);
      emitOpenEvent();
    });
    await flushAsync();

    expect(apiMocks.fetchUpstreamAccounts).toHaveBeenCalledTimes(2);
    expect(apiMocks.fetchUpstreamAccountDetail).toHaveBeenCalledTimes(2);
  } finally {
    vi.useRealTimers();
  }
});
it("does not let records SSE preempt the first roster load before the current query hydrates", async () => {
  const pendingList = deferred<UpstreamAccountListResponse>();
  apiMocks.fetchUpstreamAccounts.mockImplementationOnce(async () => pendingList.promise);
  apiMocks.fetchUpstreamAccountDetail.mockResolvedValue(createDetail(1, "Alpha"));

  render(<Probe />);

  expect(apiMocks.fetchUpstreamAccounts).toHaveBeenCalledTimes(1);

  vi.useFakeTimers();
  try {
    act(() => {
      emitRecordsEvent();
      vi.advanceTimersByTime(10_000);
    });
    await flushAsync();

    expect(apiMocks.fetchUpstreamAccounts).toHaveBeenCalledTimes(1);

    pendingList.resolve(createListResponse());
    await flushAsync();
    await flushAsync();

    expect(text("list-status")).toBe("ready");
    expect(text("selected-name")).toBe("Alpha");
    expect(apiMocks.fetchUpstreamAccounts).toHaveBeenCalledTimes(1);
  } finally {
    vi.useRealTimers();
  }
});
it("ignores same-query records SSE events even while a prior manual refresh is already running", async () => {
  const manualRefresh = deferred<UpstreamAccountListResponse>();
  apiMocks.fetchUpstreamAccountDetail.mockResolvedValue(createDetail(1, "Alpha"));

  render(<Probe />);
  await flushAsync();

  apiMocks.fetchUpstreamAccounts.mockImplementationOnce(async () => manualRefresh.promise);

  vi.useFakeTimers();
  try {
    act(() => {
      click("refresh");
    });
    await flushAsync();

    expect(apiMocks.fetchUpstreamAccounts).toHaveBeenCalledTimes(2);

    act(() => {
      emitRecordsEvent();
      emitRecordsEvent();
      vi.advanceTimersByTime(10_000);
    });
    await flushAsync();

    expect(apiMocks.fetchUpstreamAccounts).toHaveBeenCalledTimes(2);

    manualRefresh.resolve(
      createListResponse({
        items: [createSummary(1, "Alpha Refresh 1"), createSummary(2, "Beta")],
      }),
    );
    await flushAsync();
    await flushAsync();

    expect(text("selected-name")).toBe("Alpha Refresh 1");
    expect(apiMocks.fetchUpstreamAccounts).toHaveBeenCalledTimes(2);
  } finally {
    vi.useRealTimers();
  }
});
