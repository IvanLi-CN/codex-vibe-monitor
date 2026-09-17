/** @vitest-environment jsdom */
import { act } from "react";
import { expect, it, vi } from "vitest";
import type {
  InvocationRecordsNewCountResponse,
  InvocationRecordsResponse,
  InvocationRecordsSummaryResponse,
} from "../lib/api";
import { RECORDS_NEW_COUNT_POLL_INTERVAL_MS } from "../lib/invocationRecords";
import {
  apiMocks,
  click,
  createListResponse,
  createNewCountResponse,
  createRecord,
  createSummaryResponse,
  flushAsync,
  Probe,
  render,
  text,
  waitFor,
} from "./useInvocationRecords.test-support";

it("ignores stale overlapping new-count polls for the same snapshot", async () => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2026-03-10T02:00:00Z"));

  const pollResolvers: Array<(value: InvocationRecordsNewCountResponse) => void> = [];

  apiMocks.fetchInvocationRecords.mockResolvedValue(createListResponse({ snapshotId: 42 }));
  apiMocks.fetchInvocationRecordsSummary.mockResolvedValue(
    createSummaryResponse({ snapshotId: 42, newRecordsCount: 0 }),
  );
  apiMocks.fetchInvocationRecordsNewCount.mockImplementation(
    async () =>
      new Promise<InvocationRecordsNewCountResponse>((resolve) => {
        pollResolvers.push(resolve);
      }),
  );

  render(<Probe />);
  await flushAsync();

  await act(async () => {
    await vi.advanceTimersByTimeAsync(RECORDS_NEW_COUNT_POLL_INTERVAL_MS);
  });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(RECORDS_NEW_COUNT_POLL_INTERVAL_MS);
  });

  expect(apiMocks.fetchInvocationRecordsNewCount).toHaveBeenCalledTimes(2);
  expect(pollResolvers).toHaveLength(2);

  act(() => {
    pollResolvers[1](createNewCountResponse({ snapshotId: 42, newRecordsCount: 9 }));
  });
  await flushAsync();
  expect(text("new-count")).toBe("9");

  act(() => {
    pollResolvers[0](createNewCountResponse({ snapshotId: 42, newRecordsCount: 3 }));
  });
  await flushAsync();
  expect(text("new-count")).toBe("9");
});
it("shows records as soon as the list resolves even if the summary is still pending", async () => {
  let resolveSummary: ((value: InvocationRecordsSummaryResponse) => void) | null = null;
  const summaryPromise = new Promise<InvocationRecordsSummaryResponse>((resolve) => {
    resolveSummary = resolve;
  });

  apiMocks.fetchInvocationRecords.mockResolvedValue(createListResponse({ snapshotId: 42 }));
  apiMocks.fetchInvocationRecordsSummary.mockImplementation(async () => summaryPromise);
  apiMocks.fetchInvocationRecordsNewCount.mockResolvedValue(
    createNewCountResponse({ snapshotId: 42, newRecordsCount: 0 }),
  );

  render(<Probe />);
  await flushAsync();

  expect(text("snapshot")).toBe("42");
  expect(text("model")).toBe("baseline-model");
  expect(text("records-loading")).toBe("no");
  expect(text("summary-loading")).toBe("yes");

  if (!resolveSummary) {
    throw new Error("summary resolver missing");
  }
  const resolveSummaryFn = resolveSummary as (value: InvocationRecordsSummaryResponse) => void;
  resolveSummaryFn(createSummaryResponse({ snapshotId: 42, newRecordsCount: 0 }));
  await flushAsync();

  expect(text("summary-loading")).toBe("no");
  expect(text("summary-error")).toBe("");
});
it("keeps the last snapshot visible when a new search fails", async () => {
  apiMocks.fetchInvocationRecords.mockImplementation(async (query) => {
    if (query.model === "next-model") {
      throw new Error("search failed");
    }

    return createListResponse({ snapshotId: 42 });
  });
  apiMocks.fetchInvocationRecordsSummary.mockResolvedValue(
    createSummaryResponse({ snapshotId: 42, newRecordsCount: 5 }),
  );
  apiMocks.fetchInvocationRecordsNewCount.mockResolvedValue(
    createNewCountResponse({ snapshotId: 42, newRecordsCount: 0 }),
  );

  render(<Probe />);
  await flushAsync();

  expect(text("snapshot")).toBe("42");
  expect(text("summary-snapshot")).toBe("42");

  click("draft-model");
  click("search");
  await flushAsync();

  expect(text("snapshot")).toBe("42");
  expect(text("model")).toBe("baseline-model");
  expect(text("summary-snapshot")).toBe("42");
  expect(text("new-count")).toBe("0");
  expect(text("records-error")).toContain("search failed");
  expect(text("summary-error")).toBe("");
});
it("clears the old summary once a new list snapshot lands before summary resolves", async () => {
  let resolveSummary: ((value: InvocationRecordsSummaryResponse) => void) | null = null;
  const nextSummaryPromise = new Promise<InvocationRecordsSummaryResponse>((resolve) => {
    resolveSummary = resolve;
  });

  apiMocks.fetchInvocationRecords.mockImplementation(async (query) => {
    if (query.model === "next-model") {
      return createListResponse({
        snapshotId: 84,
        page: 1,
        pageSize: query.pageSize ?? 20,
        total: 1,
        records: [
          {
            id: 84,
            invokeId: "invoke-next",
            occurredAt: "2026-03-10T02:00:00Z",
            createdAt: "2026-03-10T02:00:00Z",
            model: "next-model",
            status: "success",
          },
        ],
      });
    }

    return createListResponse({ snapshotId: 42 });
  });
  apiMocks.fetchInvocationRecordsSummary.mockImplementation(async (query) => {
    if (query.snapshotId === 84) {
      return nextSummaryPromise;
    }
    return createSummaryResponse({ snapshotId: 42, newRecordsCount: 0 });
  });
  apiMocks.fetchInvocationRecordsNewCount.mockResolvedValue(
    createNewCountResponse({ snapshotId: 42, newRecordsCount: 0 }),
  );

  render(<Probe />);
  await flushAsync();

  expect(text("summary-snapshot")).toBe("42");

  click("draft-model");
  click("search");
  await flushAsync();

  expect(text("snapshot")).toBe("84");
  expect(text("summary-snapshot")).toBe("0");
  expect(text("summary-loading")).toBe("yes");

  if (!resolveSummary) {
    throw new Error("next summary resolver missing");
  }
  const resolveSummaryFn = resolveSummary as (value: InvocationRecordsSummaryResponse) => void;
  resolveSummaryFn(createSummaryResponse({ snapshotId: 84, newRecordsCount: 0 }));
  await flushAsync();

  expect(text("summary-snapshot")).toBe("84");
  expect(text("summary-loading")).toBe("no");
});
it("keeps the last summary visible when new-count polling fails", async () => {
  vi.useFakeTimers();

  apiMocks.fetchInvocationRecords.mockResolvedValue(createListResponse({ snapshotId: 42 }));
  apiMocks.fetchInvocationRecordsSummary.mockResolvedValue(
    createSummaryResponse({ snapshotId: 42, newRecordsCount: 0 }),
  );
  apiMocks.fetchInvocationRecordsNewCount.mockRejectedValue(new Error("poll failed"));

  render(<Probe />);
  await flushAsync();

  expect(text("new-count")).toBe("0");
  expect(text("summary-error")).toBe("");

  await act(async () => {
    await vi.advanceTimersByTimeAsync(RECORDS_NEW_COUNT_POLL_INTERVAL_MS);
  });
  await flushAsync();

  expect(text("new-count")).toBe("0");
  expect(text("summary-error")).toBe("");
  expect(apiMocks.fetchInvocationRecordsSummary).toHaveBeenCalledTimes(1);
  expect(apiMocks.fetchInvocationRecordsNewCount).toHaveBeenCalledTimes(1);
});
it("keeps the preserved summary alive for lightweight polling when a refreshed summary fails", async () => {
  vi.useFakeTimers();

  apiMocks.fetchInvocationRecords.mockImplementation(async (query) => {
    if (query.snapshotId === undefined) {
      return createListResponse({
        snapshotId: 84,
        total: 1,
        records: [
          {
            id: 84,
            invokeId: "invoke-refresh",
            occurredAt: "2026-03-10T02:00:00Z",
            createdAt: "2026-03-10T02:00:00Z",
            model: "baseline-refreshed",
            status: "success",
          },
        ],
      });
    }

    return createListResponse({ snapshotId: 42 });
  });
  apiMocks.fetchInvocationRecordsSummary
    .mockResolvedValueOnce(createSummaryResponse({ snapshotId: 42, newRecordsCount: 0 }))
    .mockRejectedValueOnce(new Error("summary failed"));
  apiMocks.fetchInvocationRecordsNewCount.mockResolvedValue(
    createNewCountResponse({ snapshotId: 84, newRecordsCount: 9 }),
  );

  render(<Probe />);
  await flushAsync();

  expect(text("summary-snapshot")).toBe("42");
  expect(text("new-count")).toBe("0");

  click("refresh-applied");
  await flushAsync();

  expect(text("snapshot")).toBe("84");
  expect(text("model")).toBe("baseline-refreshed");
  expect(text("summary-snapshot")).toBe("42");
  expect(text("new-count")).toBe("0");
  expect(text("summary-error")).toContain("summary failed");

  await act(async () => {
    await vi.advanceTimersByTimeAsync(RECORDS_NEW_COUNT_POLL_INTERVAL_MS);
  });
  await flushAsync();

  expect(apiMocks.fetchInvocationRecordsNewCount).toHaveBeenCalledTimes(1);
  expect(text("summary-snapshot")).toBe("42");
  expect(text("new-count")).toBe("9");
});
it("keeps records visible when an applied refresh summary fails", async () => {
  vi.useFakeTimers();

  apiMocks.fetchInvocationRecords
    .mockResolvedValueOnce(
      createListResponse({
        snapshotId: 42,
        records: [
          createRecord({
            id: 1,
            invokeId: "invoke-baseline",
            model: "baseline-model",
          }),
        ],
      }),
    )
    .mockResolvedValueOnce(
      createListResponse({
        snapshotId: 84,
        records: [
          createRecord({
            id: 84,
            invokeId: "invoke-refreshed",
            occurredAt: "2026-03-10T02:00:00Z",
            createdAt: "2026-03-10T02:00:00Z",
            model: "baseline-refreshed",
            status: "success",
          }),
        ],
      }),
    );
  apiMocks.fetchInvocationRecordsSummary
    .mockResolvedValueOnce(createSummaryResponse({ snapshotId: 42, newRecordsCount: 0 }))
    .mockRejectedValueOnce(new Error("summary refresh failed"));
  apiMocks.fetchInvocationRecordsNewCount.mockResolvedValue(
    createNewCountResponse({ snapshotId: 42, newRecordsCount: 0 }),
  );

  render(<Probe />);
  await flushAsync();

  expect(text("snapshot")).toBe("42");
  expect(text("summary-snapshot")).toBe("42");
  expect(text("records-error")).toBe("");
  expect(text("summary-error")).toBe("");

  click("refresh-applied");
  await flushAsync();

  await waitFor(() => {
    expect(apiMocks.fetchInvocationRecords).toHaveBeenCalledTimes(2);
  });

  expect(text("snapshot")).toBe("84");
  expect(text("model")).toBe("baseline-refreshed");
  expect(text("records-error")).toBe("");
  expect(text("summary-snapshot")).toBe("42");
  expect(text("summary-error")).toContain("summary refresh failed");
});
it("keeps the previous page size when a page-size request fails before search", async () => {
  apiMocks.fetchInvocationRecords.mockImplementation(async (query) => {
    if (query.model === "next-model") {
      return createListResponse({
        snapshotId: 84,
        page: 1,
        pageSize: query.pageSize ?? 20,
        total: 1,
        records: [
          {
            id: 84,
            invokeId: "invoke-next",
            occurredAt: "2026-03-10T02:00:00Z",
            createdAt: "2026-03-10T02:00:00Z",
            model: "next-model",
            status: "success",
          },
        ],
      });
    }

    if (query.snapshotId === 42 && query.pageSize === 50) {
      throw new Error("page size failed");
    }

    return createListResponse({ snapshotId: 42 });
  });

  apiMocks.fetchInvocationRecordsSummary.mockImplementation(async (query) =>
    createSummaryResponse({ snapshotId: query.snapshotId ?? 42, newRecordsCount: 0 }),
  );
  apiMocks.fetchInvocationRecordsNewCount.mockResolvedValue(
    createNewCountResponse({ snapshotId: 42, newRecordsCount: 0 }),
  );

  render(<Probe />);
  await flushAsync();

  click("page-size-50");
  await flushAsync();

  expect(text("page-size")).toBe("20");

  click("draft-model");
  click("search");
  await flushAsync();

  expect(text("snapshot")).toBe("84");
  expect(text("page-size")).toBe("20");

  const searchQuery = apiMocks.fetchInvocationRecords.mock.calls.at(-1)?.[0];
  expect(searchQuery?.model).toBe("next-model");
  expect(searchQuery?.pageSize).toBe(20);
});
it("ignores stale page-size responses that race with a newer search snapshot", async () => {
  let resolveSearch: ((value: InvocationRecordsResponse) => void) | null = null;
  let resolveOldPageSize: ((value: InvocationRecordsResponse) => void) | null = null;
  const searchPromise = new Promise<InvocationRecordsResponse>((resolve) => {
    resolveSearch = resolve;
  });
  const oldPageSizePromise = new Promise<InvocationRecordsResponse>((resolve) => {
    resolveOldPageSize = resolve;
  });

  apiMocks.fetchInvocationRecords.mockImplementation(async (query) => {
    if (query.model === "next-model") {
      return searchPromise;
    }

    if (query.snapshotId === 42 && query.pageSize === 50) {
      return oldPageSizePromise;
    }

    return createListResponse({ snapshotId: 42 });
  });

  apiMocks.fetchInvocationRecordsSummary.mockImplementation(async (query) =>
    createSummaryResponse({ snapshotId: query.snapshotId ?? 42, newRecordsCount: 0 }),
  );
  apiMocks.fetchInvocationRecordsNewCount.mockResolvedValue(
    createNewCountResponse({ snapshotId: 42, newRecordsCount: 0 }),
  );

  render(<Probe />);
  await flushAsync();

  expect(text("page-size")).toBe("20");

  click("draft-model");
  click("search");
  await flushAsync();
  click("page-size-50");
  await flushAsync();

  if (!resolveSearch || !resolveOldPageSize) {
    throw new Error("missing async resolver");
  }
  const resolveSearchFn = resolveSearch as (value: InvocationRecordsResponse) => void;
  const resolveOldPageSizeFn = resolveOldPageSize as (value: InvocationRecordsResponse) => void;

  resolveSearchFn(
    createListResponse({
      snapshotId: 84,
      total: 1,
      page: 1,
      pageSize: 20,
      records: [
        {
          id: 84,
          invokeId: "invoke-next",
          occurredAt: "2026-03-10T02:00:00Z",
          createdAt: "2026-03-10T02:00:00Z",
          model: "next-model",
          status: "success",
        },
      ],
    }),
  );
  await flushAsync();

  expect(text("snapshot")).toBe("84");
  expect(text("model")).toBe("next-model");
  expect(text("page-size")).toBe("20");

  resolveOldPageSizeFn(
    createListResponse({
      snapshotId: 42,
      page: 1,
      pageSize: 50,
      records: [
        {
          id: 2,
          invokeId: "invoke-old",
          occurredAt: "2026-03-10T00:05:00Z",
          createdAt: "2026-03-10T00:05:00Z",
          model: "baseline-model",
          status: "failed",
        },
      ],
    }),
  );
  await flushAsync();

  expect(text("snapshot")).toBe("84");
  expect(text("model")).toBe("next-model");
  expect(text("page-size")).toBe("20");
});
it("ignores stale page responses once a new search starts", async () => {
  let resolvePageTwo: ((value: InvocationRecordsResponse) => void) | null = null;
  const pageTwoPromise = new Promise<InvocationRecordsResponse>((resolve) => {
    resolvePageTwo = resolve;
  });

  apiMocks.fetchInvocationRecords.mockImplementation(async (query) => {
    if (query.snapshotId === 42 && query.page === 2) {
      return pageTwoPromise;
    }

    if (query.model === "next-model") {
      return createListResponse({
        snapshotId: 84,
        total: 1,
        page: 1,
        pageSize: query.pageSize ?? 20,
        records: [
          {
            id: 84,
            invokeId: "invoke-next",
            occurredAt: "2026-03-10T02:00:00Z",
            createdAt: "2026-03-10T02:00:00Z",
            model: "next-model",
            status: "success",
          },
        ],
      });
    }

    return createListResponse({ snapshotId: 42 });
  });

  apiMocks.fetchInvocationRecordsSummary.mockImplementation(async (query) =>
    createSummaryResponse({ snapshotId: query.snapshotId ?? 42, newRecordsCount: 4 }),
  );
  apiMocks.fetchInvocationRecordsNewCount.mockResolvedValue(
    createNewCountResponse({ snapshotId: 42, newRecordsCount: 0 }),
  );

  render(<Probe />);
  await flushAsync();

  click("page-2");
  click("draft-model");
  click("search");
  await flushAsync();

  expect(text("snapshot")).toBe("84");
  expect(text("page")).toBe("1");
  expect(text("model")).toBe("next-model");
  expect(text("new-count")).toBe("0");

  if (!resolvePageTwo) {
    throw new Error("page two resolver missing");
  }
  const resolvePageTwoFn = resolvePageTwo as (value: InvocationRecordsResponse) => void;

  resolvePageTwoFn(
    createListResponse({
      snapshotId: 42,
      page: 2,
      pageSize: 20,
      records: [
        {
          id: 2,
          invokeId: "invoke-2",
          occurredAt: "2026-03-10T00:05:00Z",
          createdAt: "2026-03-10T00:05:00Z",
          model: "baseline-model",
          status: "failed",
        },
      ],
    }),
  );
  await flushAsync();

  expect(text("snapshot")).toBe("84");
  expect(text("page")).toBe("1");
  expect(text("model")).toBe("next-model");
});
