/** @vitest-environment jsdom */
import { act } from "react";
import { expect, it } from "vitest";
import type {
  UpstreamAccountDetail,
  UpstreamAccountListResponse,
  UpstreamAccountWindowUsageResponse,
} from "../lib/api";
import {
  apiMocks,
  click,
  createDetail,
  createForwardProxyNode,
  createGroupSummary,
  createListResponse,
  createSummary,
  createWindowedSummary,
  createWindowUsageResponse,
  deferred,
  flushAsync,
  host,
  Probe,
  render,
  rerender,
  text,
} from "./useUpstreamAccounts.test-support";

it("defers the roster request until the query is available", async () => {
  render(<Probe query={null} />);
  await flushAsync();

  expect(apiMocks.fetchUpstreamAccounts).not.toHaveBeenCalled();
});
it("passes server-side roster filters through to the list endpoint", async () => {
  render(
    <Probe
      query={{
        groupSearch: "prod",
        workStatus: ["working", "rate_limited"],
        healthStatus: ["normal"],
        tagIds: [1, 2],
      }}
    />,
  );
  await flushAsync();

  expect(apiMocks.fetchUpstreamAccounts).toHaveBeenCalledWith({
    groupSearch: "prod",
    workStatus: ["working", "rate_limited"],
    healthStatus: ["normal"],
    tagIds: [1, 2],
  });
});
it("treats grouped includeAll queries as a distinct roster key", async () => {
  apiMocks.fetchUpstreamAccountDetail.mockResolvedValue(createDetail(1, "Alpha"));

  render(<Probe query={{ page: 1, pageSize: 20 }} />);
  await flushAsync();

  rerender(<Probe query={{ includeAll: true }} />);
  await flushAsync();

  expect(apiMocks.fetchUpstreamAccounts).toHaveBeenNthCalledWith(1, {
    page: 1,
    pageSize: 20,
  });
  expect(apiMocks.fetchUpstreamAccounts).toHaveBeenNthCalledWith(2, {
    includeAll: true,
  });
});
it("auto-hydrates window usage only for the selected account", async () => {
  const hydration = deferred<UpstreamAccountWindowUsageResponse>();
  apiMocks.fetchUpstreamAccounts.mockResolvedValueOnce(
    createListResponse({
      items: [createWindowedSummary(1, "Alpha"), createWindowedSummary(2, "Beta")],
    }),
  );
  apiMocks.fetchUpstreamAccountWindowUsage.mockImplementationOnce(async () => hydration.promise);

  render(<Probe query={{ page: 1, pageSize: 20 }} />);
  await flushAsync();

  expect(text("window-usage-pending")).toBe("true");
  expect(text("first-item-primary-requests")).toBe("");
  expect(apiMocks.fetchUpstreamAccountWindowUsage).toHaveBeenCalledWith([1]);

  hydration.resolve(createWindowUsageResponse([1]));
  await flushAsync();

  expect(text("window-usage-pending")).toBe("false");
  expect(text("first-item-primary-requests")).toBe("10");
  expect(text("first-item-secondary-requests")).toBe("30");
});
it("hydrates only the selected account for includeAll roster queries", async () => {
  apiMocks.fetchUpstreamAccounts.mockResolvedValueOnce(
    createListResponse({
      items: [createWindowedSummary(1, "Alpha"), createWindowedSummary(2, "Beta")],
    }),
  );
  apiMocks.fetchUpstreamAccountWindowUsage.mockResolvedValueOnce(createWindowUsageResponse([1, 2]));

  render(<Probe query={{ includeAll: true }} />);
  await flushAsync();

  expect(apiMocks.fetchUpstreamAccountWindowUsage).toHaveBeenCalledWith([1]);
  expect(text("window-usage-pending")).toBe("false");
  expect(text("first-item-primary-requests")).toBe("10");

  click("hydrate-visible");
  await flushAsync();
  await flushAsync();

  expect(apiMocks.fetchUpstreamAccountWindowUsage).toHaveBeenCalledWith([2]);
  expect(text("first-item-primary-requests")).toBe("10");
});
it("auto-hydrates visible roster rows when the query opts out of default selection", async () => {
  apiMocks.fetchUpstreamAccounts.mockResolvedValueOnce(
    createListResponse({
      items: [createWindowedSummary(1, "Alpha"), createWindowedSummary(2, "Beta")],
    }),
  );
  render(<Probe query={{ includeAll: true }} options={{ fallbackToFirstItem: false }} />);
  await flushAsync();

  expect(text("selected-id")).toBe("");
  expect(apiMocks.fetchUpstreamAccountWindowUsage).not.toHaveBeenCalled();
  expect(text("window-usage-pending")).toBe("false");
  expect(text("first-item-primary-requests")).toBe("");
  expect(text("first-item-secondary-requests")).toBe("");

  apiMocks.fetchUpstreamAccountWindowUsage.mockResolvedValueOnce(createWindowUsageResponse([1, 2]));
  click("hydrate-visible");
  await flushAsync();
  await flushAsync();

  expect(apiMocks.fetchUpstreamAccountWindowUsage).toHaveBeenCalledWith([1, 2]);
  expect(text("first-item-primary-requests")).toBe("10");
  expect(text("first-item-secondary-requests")).toBe("30");
});
it("drops stale selected-account window-usage responses after the roster query changes", async () => {
  const firstHydration = deferred<UpstreamAccountWindowUsageResponse>();
  apiMocks.fetchUpstreamAccounts
    .mockResolvedValueOnce(
      createListResponse({
        items: [createWindowedSummary(1, "Alpha"), createWindowedSummary(2, "Beta")],
      }),
    )
    .mockResolvedValueOnce(
      createListResponse({
        items: [createWindowedSummary(3, "Gamma"), createWindowedSummary(4, "Delta")],
        total: 4,
        page: 2,
        pageSize: 20,
      }),
    );
  apiMocks.fetchUpstreamAccountWindowUsage.mockImplementationOnce(
    async () => firstHydration.promise,
  );

  render(<Probe query={{ page: 1, pageSize: 20 }} />);
  await flushAsync();

  expect(apiMocks.fetchUpstreamAccountWindowUsage).toHaveBeenCalledWith([1]);
  expect(text("window-usage-pending")).toBe("true");

  rerender(<Probe query={{ includeAll: true }} />);
  await flushAsync();

  expect(text("first-item-id")).toBe("3");
  expect(text("window-usage-pending")).toBe("false");
  expect(text("first-item-primary-requests")).toBe("");

  firstHydration.resolve(createWindowUsageResponse([1]));
  await flushAsync();

  expect(text("first-item-id")).toBe("3");
  expect(text("first-item-primary-requests")).toBe("");
  expect(text("first-item-secondary-requests")).toBe("");
});
it("keeps selected-account window-usage pending while a manual rehydrate supersedes an older request", async () => {
  const firstHydration = deferred<UpstreamAccountWindowUsageResponse>();
  const secondHydration = deferred<UpstreamAccountWindowUsageResponse>();
  apiMocks.fetchUpstreamAccounts
    .mockResolvedValueOnce(
      createListResponse({
        items: [createWindowedSummary(1, "Alpha"), createWindowedSummary(2, "Beta")],
      }),
    )
    .mockResolvedValueOnce(
      createListResponse({
        items: [createWindowedSummary(1, "Alpha"), createWindowedSummary(2, "Beta")],
      }),
    );
  apiMocks.fetchUpstreamAccountWindowUsage
    .mockImplementationOnce(async () => firstHydration.promise)
    .mockImplementationOnce(async () => secondHydration.promise)
    .mockResolvedValueOnce(createWindowUsageResponse([2]));

  render(<Probe query={{ page: 1, pageSize: 20 }} />);
  await flushAsync();

  expect(apiMocks.fetchUpstreamAccountWindowUsage).toHaveBeenNthCalledWith(1, [1]);
  expect(text("window-usage-pending")).toBe("true");

  click("hydrate-visible");
  await flushAsync();

  expect(apiMocks.fetchUpstreamAccountWindowUsage).toHaveBeenNthCalledWith(2, [2]);
  expect(text("window-usage-pending")).toBe("true");

  firstHydration.resolve(createWindowUsageResponse([1]));
  await flushAsync();

  expect(text("window-usage-pending")).toBe("true");
  expect(text("first-item-primary-requests")).toBe("10");

  secondHydration.resolve(createWindowUsageResponse([2]));
  await flushAsync();

  expect(text("window-usage-pending")).toBe("false");
  expect(text("first-item-primary-requests")).toBe("10");
  expect(text("first-item-secondary-requests")).toBe("30");
});
it("marks a query switch as stale until the new roster lands", async () => {
  const nextPage = deferred<UpstreamAccountListResponse>();
  apiMocks.fetchUpstreamAccounts
    .mockResolvedValueOnce(createListResponse())
    .mockImplementationOnce(async () => nextPage.promise);
  apiMocks.fetchUpstreamAccountDetail.mockResolvedValue(createDetail(1, "Alpha"));

  render(<Probe query={{ page: 1, pageSize: 20 }} />);
  await flushAsync();

  expect(text("list-freshness")).toBe("fresh");
  expect(text("list-loading-state")).toBe("idle");
  expect(text("list-status")).toBe("ready");
  expect(text("list-has-current-query-data")).toBe("true");

  rerender(<Probe query={{ page: 2, pageSize: 20 }} />);
  await flushAsync();

  expect(text("selected-name")).toBe("Alpha");
  expect(text("list-freshness")).toBe("stale");
  expect(text("list-loading-state")).toBe("switching");
  expect(text("list-status")).toBe("loading");
  expect(text("list-has-current-query-data")).toBe("false");

  nextPage.resolve({
    ...createListResponse(),
    items: [createSummary(3, "Gamma"), createSummary(4, "Delta")],
    total: 4,
    page: 2,
    pageSize: 20,
  });
  await flushAsync();

  expect(text("selected-id")).toBe("3");
  expect(text("selected-name")).toBe("Gamma");
  expect(text("list-freshness")).toBe("fresh");
  expect(text("list-loading-state")).toBe("idle");
  expect(text("list-status")).toBe("ready");
  expect(text("list-has-current-query-data")).toBe("true");
});
it("does not refetch the roster when rerenders keep the same query key", async () => {
  apiMocks.fetchUpstreamAccountDetail.mockResolvedValue(createDetail(1, "Alpha"));

  render(<Probe query={{ page: 1, pageSize: 20 }} />);
  await flushAsync();

  expect(apiMocks.fetchUpstreamAccounts).toHaveBeenCalledTimes(1);

  rerender(<Probe query={{ page: 1, pageSize: 20 }} />);
  await flushAsync();

  expect(apiMocks.fetchUpstreamAccounts).toHaveBeenCalledTimes(1);
});
it("keeps the forward proxy catalog in loading state until the first roster payload lands", async () => {
  const listRequest = deferred<UpstreamAccountListResponse>();
  apiMocks.fetchUpstreamAccounts.mockReturnValueOnce(listRequest.promise);

  render(<Probe />);

  expect(text("proxy-catalog-kind")).toBe("loading");
  expect(text("proxy-catalog-freshness")).toBe("missing");

  listRequest.resolve(
    createListResponse({
      forwardProxyNodes: [createForwardProxyNode("jp-edge-01")],
    }),
  );
  await flushAsync();

  expect(text("proxy-catalog-kind")).toBe("ready-with-data");
  expect(text("proxy-catalog-freshness")).toBe("fresh");
});
it("reports an empty-but-loaded forward proxy catalog distinctly from loading", async () => {
  apiMocks.fetchUpstreamAccounts.mockResolvedValueOnce(
    createListResponse({ forwardProxyNodes: [] }),
  );

  render(<Probe />);
  await flushAsync();

  expect(text("proxy-catalog-kind")).toBe("ready-empty");
  expect(text("proxy-catalog-freshness")).toBe("fresh");
});
it("treats a pending refresh of an empty proxy catalog as loading until the refreshed roster lands", async () => {
  const refreshedList = deferred<UpstreamAccountListResponse>();
  apiMocks.fetchUpstreamAccounts
    .mockResolvedValueOnce(createListResponse({ forwardProxyNodes: [] }))
    .mockImplementationOnce(async () => refreshedList.promise);

  render(<Probe />);
  await flushAsync();

  expect(text("proxy-catalog-kind")).toBe("ready-empty");
  expect(text("proxy-catalog-freshness")).toBe("fresh");

  act(() => {
    (host?.querySelector('[data-testid="refresh"]') as HTMLButtonElement | null)?.click();
  });
  await flushAsync();

  expect(text("proxy-catalog-kind")).toBe("loading");
  expect(text("proxy-catalog-freshness")).toBe("stale");

  refreshedList.resolve(
    createListResponse({
      forwardProxyNodes: [createForwardProxyNode("jp-edge-01")],
    }),
  );
  await flushAsync();

  expect(text("proxy-catalog-kind")).toBe("ready-with-data");
  expect(text("proxy-catalog-freshness")).toBe("fresh");
});
it("keeps an empty proxy catalog stale after a refresh failure", async () => {
  apiMocks.fetchUpstreamAccounts
    .mockResolvedValueOnce(createListResponse({ forwardProxyNodes: [] }))
    .mockRejectedValueOnce(new Error("refresh failed"));

  render(<Probe />);
  await flushAsync();

  expect(text("proxy-catalog-kind")).toBe("ready-empty");
  expect(text("proxy-catalog-freshness")).toBe("fresh");

  act(() => {
    (host?.querySelector('[data-testid="refresh"]') as HTMLButtonElement | null)?.click();
  });
  await flushAsync();

  expect(text("list-error")).toBe("refresh failed");
  expect(text("proxy-catalog-kind")).toBe("ready-empty");
  expect(text("proxy-catalog-freshness")).toBe("stale");
});
it("keeps a populated proxy catalog stale after a refresh failure", async () => {
  apiMocks.fetchUpstreamAccounts
    .mockResolvedValueOnce(
      createListResponse({
        forwardProxyNodes: [createForwardProxyNode("jp-edge-01")],
      }),
    )
    .mockRejectedValueOnce(new Error("refresh failed"));

  render(<Probe />);
  await flushAsync();

  expect(text("proxy-catalog-kind")).toBe("ready-with-data");
  expect(text("proxy-catalog-freshness")).toBe("fresh");

  act(() => {
    (host?.querySelector('[data-testid="refresh"]') as HTMLButtonElement | null)?.click();
  });
  await flushAsync();

  expect(text("list-error")).toBe("refresh failed");
  expect(text("proxy-catalog-kind")).toBe("ready-with-data");
  expect(text("proxy-catalog-freshness")).toBe("stale");
});
it("reports the current query as failed after a switched roster request rejects", async () => {
  const nextPage = deferred<UpstreamAccountListResponse>();
  apiMocks.fetchUpstreamAccounts
    .mockResolvedValueOnce(createListResponse())
    .mockImplementationOnce(async () => nextPage.promise);
  apiMocks.fetchUpstreamAccountDetail.mockResolvedValue(createDetail(1, "Alpha"));

  render(<Probe query={{ page: 1, pageSize: 20 }} />);
  await flushAsync();

  rerender(<Probe query={{ page: 2, pageSize: 20 }} />);
  await flushAsync();

  nextPage.reject(new Error("page two failed"));
  await flushAsync();

  expect(text("selected-name")).toBe("Alpha");
  expect(text("list-error")).toBe("page two failed");
  expect(text("list-freshness")).toBe("stale");
  expect(text("list-loading-state")).toBe("idle");
  expect(text("list-status")).toBe("error");
  expect(text("list-has-current-query-data")).toBe("false");
});
it("ignores stale detail responses after account switches", async () => {
  const first = deferred<UpstreamAccountDetail>();
  const second = deferred<UpstreamAccountDetail>();
  apiMocks.fetchUpstreamAccountDetail
    .mockImplementationOnce(async () => first.promise)
    .mockImplementationOnce(async () => second.promise);

  render(<Probe />);
  await flushAsync();

  expect(text("selected-id")).toBe("1");
  click("select-beta");
  await flushAsync();

  second.resolve(createDetail(2, "Beta"));
  await flushAsync();
  expect(text("detail-id")).toBe("2");
  expect(text("detail-name")).toBe("Beta");

  first.resolve(createDetail(1, "Alpha"));
  await flushAsync();
  expect(text("selected-id")).toBe("2");
  expect(text("detail-id")).toBe("2");
  expect(text("detail-name")).toBe("Beta");
});
it("ignores stale detail errors after account switches", async () => {
  const first = deferred<UpstreamAccountDetail>();
  const second = deferred<UpstreamAccountDetail>();
  apiMocks.fetchUpstreamAccountDetail
    .mockImplementationOnce(async () => first.promise)
    .mockImplementationOnce(async () => second.promise);

  render(<Probe />);
  await flushAsync();

  click("select-beta");
  await flushAsync();

  second.resolve(createDetail(2, "Beta"));
  await flushAsync();

  first.reject(new Error("Alpha failed"));
  await flushAsync();

  expect(text("selected-id")).toBe("2");
  expect(text("detail-id")).toBe("2");
  expect(text("detail-name")).toBe("Beta");
  expect(text("error")).toBe("");
});
it("invalidates the previous detail request in the same turn as a selection change", async () => {
  const first = deferred<UpstreamAccountDetail>();
  const second = deferred<UpstreamAccountDetail>();
  apiMocks.fetchUpstreamAccountDetail
    .mockImplementationOnce(async () => first.promise)
    .mockImplementationOnce(async () => second.promise);

  render(<Probe />);
  await flushAsync();

  click("select-beta");
  first.reject(new Error("Alpha failed"));
  await flushAsync();

  second.resolve(createDetail(2, "Beta"));
  await flushAsync();

  expect(text("selected-id")).toBe("2");
  expect(text("detail-id")).toBe("2");
  expect(text("detail-name")).toBe("Beta");
  expect(text("error")).toBe("");
});
it("does not reclaim selection when an older account sync finishes later", async () => {
  const sync = deferred<UpstreamAccountDetail>();
  apiMocks.fetchUpstreamAccountDetail
    .mockResolvedValueOnce(createDetail(1, "Alpha"))
    .mockResolvedValueOnce(createDetail(2, "Beta"))
    .mockResolvedValue(createDetail(2, "Beta"));
  apiMocks.syncUpstreamAccount.mockImplementationOnce(async () => sync.promise);

  render(<Probe />);
  await flushAsync();

  expect(text("selected-id")).toBe("1");
  expect(text("detail-id")).toBe("1");

  click("sync-alpha");
  click("select-beta");
  await flushAsync();
  expect(text("selected-id")).toBe("2");

  sync.resolve(createDetail(1, "Alpha"));
  await flushAsync();

  expect(text("selected-id")).toBe("2");
  expect(text("selected-name")).toBe("Beta");
  expect(text("detail-id")).not.toBe("1");
  expect(text("detail-name")).not.toBe("Alpha");
});
it("reloads the currently selected account detail after another account sync finishes", async () => {
  const sync = deferred<UpstreamAccountDetail>();
  apiMocks.fetchUpstreamAccounts
    .mockResolvedValueOnce(createListResponse())
    .mockResolvedValueOnce(createListResponse());
  apiMocks.fetchUpstreamAccountDetail
    .mockResolvedValueOnce(createDetail(1, "Alpha"))
    .mockResolvedValueOnce(createDetail(2, "Beta Stale"))
    .mockResolvedValueOnce(createDetail(2, "Beta Fresh"));
  apiMocks.syncUpstreamAccount.mockImplementationOnce(async () => sync.promise);

  render(<Probe />);
  await flushAsync();

  click("sync-alpha");
  click("select-beta");
  await flushAsync();
  expect(text("selected-id")).toBe("2");
  expect(text("detail-name")).toBe("Beta Stale");

  sync.resolve(createDetail(1, "Alpha Synced"));
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
});
it("reloads the currently selected account detail after group settings save", async () => {
  apiMocks.fetchUpstreamAccounts
    .mockResolvedValueOnce(createListResponse())
    .mockResolvedValueOnce(createListResponse());
  apiMocks.fetchUpstreamAccountDetail
    .mockResolvedValueOnce(createDetail(1, "Alpha"))
    .mockResolvedValue(createDetail(1, "Alpha Group Policy Fresh"));
  apiMocks.updateUpstreamAccountGroup.mockResolvedValueOnce(
    createGroupSummary("prod", {
      routingRule: {
        allowCutOut: false,
        allowCutIn: false,
        priorityTier: "fallback",
      },
    }),
  );

  render(<Probe />);
  await flushAsync();

  expect(text("detail-name")).toBe("Alpha");
  click("save-prod-group");
  await flushAsync();

  expect(apiMocks.updateUpstreamAccountGroup).toHaveBeenCalledWith("prod", {
    routingRule: { priorityTier: "fallback" },
  });
  expect(apiMocks.fetchUpstreamAccountDetail).toHaveBeenNthCalledWith(
    2,
    1,
    expect.objectContaining({
      signal: expect.any(AbortSignal),
      includeRecentActions: undefined,
    }),
  );
  await flushAsync();
  await flushAsync();
  expect(text("detail-name")).toBe("Alpha Group Policy Fresh");
});
it("keeps synced detail when an older detail refresh resolves afterwards", async () => {
  const refreshedDetail = deferred<UpstreamAccountDetail>();
  const sync = deferred<UpstreamAccountDetail>();
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
  expect(text("detail-name")).toBe("Alpha Synced");

  refreshedDetail.resolve(createDetail(1, "Alpha Stale"));
  await flushAsync();
  expect(text("detail-name")).toBe("Alpha Synced");
});
it("does not clear the current account error when another account sync succeeds", async () => {
  const betaFailure = deferred<UpstreamAccountDetail>();
  const betaRefresh = deferred<UpstreamAccountDetail>();
  apiMocks.fetchUpstreamAccountDetail
    .mockResolvedValueOnce(createDetail(1, "Alpha"))
    .mockImplementationOnce(async () => betaFailure.promise)
    .mockImplementationOnce(async () => betaRefresh.promise);
  apiMocks.syncUpstreamAccount.mockResolvedValueOnce(createDetail(1, "Alpha Synced"));

  render(<Probe />);
  await flushAsync();

  click("select-beta");
  await flushAsync();
  betaFailure.reject(new Error("Beta failed"));
  await flushAsync();
  expect(text("selected-id")).toBe("2");
  expect(text("error")).toBe("Beta failed");

  click("sync-alpha");
  await flushAsync();

  expect(text("selected-id")).toBe("2");
  expect(text("error")).toBe("Beta failed");
});
it("refreshes detail using the list's final selection", async () => {
  const betaDetail = deferred<UpstreamAccountDetail>();
  apiMocks.fetchUpstreamAccounts.mockResolvedValueOnce(createListResponse()).mockResolvedValueOnce({
    writesEnabled: true,
    items: [createSummary(2, "Beta")],
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
    .mockImplementationOnce(async (accountId: number) => {
      if (accountId !== 2) {
        throw new Error(`unexpected account ${accountId}`);
      }
      return betaDetail.promise;
    })
    .mockResolvedValue(createDetail(2, "Beta"));

  render(<Probe />);
  await flushAsync();

  click("refresh");
  await flushAsync();

  expect(apiMocks.fetchUpstreamAccountDetail).toHaveBeenNthCalledWith(
    2,
    2,
    expect.objectContaining({
      signal: expect.any(AbortSignal),
      includeRecentActions: undefined,
    }),
  );

  betaDetail.resolve(createDetail(2, "Beta"));
  await flushAsync();

  expect(text("selected-id")).toBe("2");
  expect(text("detail-id")).toBe("2");
  expect(text("detail-name")).toBe("Beta");
  expect(text("error")).toBe("");
});
it("keeps the current detail when list refresh fails", async () => {
  apiMocks.fetchUpstreamAccounts
    .mockResolvedValueOnce(createListResponse())
    .mockRejectedValueOnce(new Error("List failed"));
  apiMocks.fetchUpstreamAccountDetail.mockResolvedValue(createDetail(1, "Alpha"));

  render(<Probe />);
  await flushAsync();

  expect(text("selected-id")).toBe("1");
  expect(text("detail-id")).toBe("1");
  expect(text("detail-name")).toBe("Alpha");

  click("refresh");
  await flushAsync();

  expect(text("selected-id")).toBe("1");
  expect(text("detail-id")).toBe("1");
  expect(text("detail-name")).toBe("Alpha");
  expect(text("error")).toBe("List failed");
});
it("keeps list and detail errors visible independently", async () => {
  apiMocks.fetchUpstreamAccountDetail
    .mockResolvedValueOnce(createDetail(1, "Alpha"))
    .mockRejectedValueOnce(new Error("Beta failed"));
  apiMocks.fetchUpstreamAccounts
    .mockResolvedValueOnce(createListResponse())
    .mockRejectedValueOnce(new Error("List failed"));

  render(<Probe />);
  await flushAsync();

  click("select-beta");
  await flushAsync();

  expect(text("detail-error")).toBe("Beta failed");
  expect(text("list-error")).toBe("");

  click("refresh");
  await flushAsync();

  expect(text("detail-error")).toBe("Beta failed");
  expect(text("list-error")).toBe("List failed");
});
