/** @vitest-environment jsdom */

import { fireEvent, waitFor } from "@testing-library/dom";
import { act } from "react";
import { expect, it, vi } from "vitest";
import {
  createConversation,
  createPreview,
  createResponse,
  createUpstreamAccountActivityResponse,
  host,
  renderSection,
  requireTestValue,
  rerenderSection,
  upstreamAccountActivityMock,
} from "./DashboardWorkingConversationsSection.support";

it("keeps the virtualized viewport spanning the full responsive grid width", () => {
  renderSection(
    createResponse([
      createConversation("pck-layout", [
        createPreview({
          id: 1,
          invokeId: "invoke-layout",
          occurredAt: "2026-04-04T10:04:00Z",
          status: "completed",
        }),
      ]),
    ]),
  );

  const grid = host?.querySelector('[data-testid="dashboard-working-conversations-grid"]');
  if (!(grid instanceof HTMLDivElement)) {
    throw new Error("missing working conversations grid");
  }
  const viewport = grid.firstElementChild;
  if (!(viewport instanceof HTMLDivElement)) {
    throw new Error("missing virtualized viewport");
  }

  expect(viewport.className).toContain("col-span-full");
});
it("rebinds width observation after the grid first mounts so wide layouts keep multi-column rendering", () => {
  const observe = vi.fn();
  const disconnect = vi.fn();
  vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(1700);
  globalThis.ResizeObserver = class {
    observe = observe;
    disconnect = disconnect;
  } as unknown as typeof ResizeObserver;

  renderSection(createResponse([]));
  rerenderSection(
    createResponse([
      createConversation("pck-wide-mounted", [
        createPreview({
          id: 1,
          invokeId: "invoke-wide-mounted",
          occurredAt: "2026-04-04T10:04:00Z",
          status: "completed",
        }),
      ]),
    ]),
  );

  const grid = host?.querySelector('[data-testid="dashboard-working-conversations-grid"]');
  if (!(grid instanceof HTMLDivElement)) {
    throw new Error("missing working conversations grid");
  }

  const rowGrid = grid.querySelector('[data-testid="dashboard-working-conversations-row"] > div');
  if (!(rowGrid instanceof HTMLDivElement)) {
    throw new Error("missing row grid");
  }

  expect(observe).toHaveBeenCalledWith(grid);
  expect(rowGrid.style.gridTemplateColumns).toBe("repeat(4, minmax(0, 1fr))");
  expect(disconnect).not.toHaveBeenCalled();
});
it("prefers the resolved CSS grid track count over the narrower container width fallback", () => {
  vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(1570);
  const originalGetComputedStyle = window.getComputedStyle.bind(window);
  vi.spyOn(window, "getComputedStyle").mockImplementation((element) => {
    const styles = originalGetComputedStyle(element);
    if (
      element instanceof HTMLElement &&
      element.dataset.testid === "dashboard-working-conversations-grid"
    ) {
      const mockedStyles = Object.create(styles) as CSSStyleDeclaration;
      Object.defineProperty(mockedStyles, "gridTemplateColumns", {
        configurable: true,
        value: "1fr 1fr 1fr 1fr",
      });
      return mockedStyles;
    }
    return styles;
  });

  renderSection(
    createResponse([
      createConversation("pck-css-track-count", [
        createPreview({
          id: 1,
          invokeId: "invoke-css-track-count",
          occurredAt: "2026-04-04T10:04:00Z",
          status: "completed",
        }),
        createPreview({
          id: 2,
          invokeId: "invoke-css-track-count-prev",
          occurredAt: "2026-04-04T10:03:30Z",
          status: "completed",
        }),
      ]),
    ]),
  );

  const rowGrid = host?.querySelector('[data-testid="dashboard-working-conversations-row"] > div');
  if (!(rowGrid instanceof HTMLDivElement)) {
    throw new Error("missing row grid");
  }

  expect(rowGrid.style.gridTemplateColumns).toBe("repeat(4, minmax(0, 1fr))");
});
it("keeps a vertical gutter between virtualized rows", () => {
  vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(1700);

  renderSection(
    createResponse(
      Array.from({ length: 8 }, (_, index) =>
        createConversation(`pck-vertical-gap-${index + 1}`, [
          createPreview({
            id: index + 1,
            invokeId: `invoke-vertical-gap-${index + 1}`,
            occurredAt: `2026-04-04T10:${String(59 - index).padStart(2, "0")}:00Z`,
            status: "completed",
          }),
        ]),
      ),
    ),
  );

  const rows = host?.querySelectorAll<HTMLElement>(
    '[data-testid="dashboard-working-conversations-row"]',
  );
  if (!rows || rows.length < 2) {
    throw new Error("expected at least two virtualized rows");
  }

  expect(rows[0]?.style.paddingBottom).toBe("16px");
  expect(rows[1]?.style.paddingBottom).toBe("0px");
});
it("does not auto-load another page on first paint when the initial grid already overflows", () => {
  vi.useFakeTimers();
  vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(1700);
  vi.spyOn(window, "innerHeight", "get").mockReturnValue(700);
  vi.spyOn(document.documentElement, "scrollHeight", "get").mockReturnValue(1680);
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
    this: HTMLElement,
  ) {
    if (this.getAttribute("data-testid") === "dashboard-working-conversations-grid") {
      return {
        x: 0,
        y: 0,
        top: 0,
        bottom: 1480,
        left: 0,
        right: 1200,
        width: 1200,
        height: 1480,
        toJSON: () => ({}),
      } satisfies DOMRect;
    }
    return {
      x: 0,
      y: 0,
      top: 0,
      bottom: 0,
      left: 0,
      right: 0,
      width: 0,
      height: 0,
      toJSON: () => ({}),
    } satisfies DOMRect;
  });
  const onLoadMore = vi.fn();

  renderSection(
    createResponse(
      Array.from({ length: 20 }, (_, index) =>
        createConversation(`pck-overflow-${index + 1}`, [
          createPreview({
            id: index + 1,
            invokeId: `invoke-overflow-${index + 1}`,
            occurredAt: `2026-04-04T10:${String(59 - index).padStart(2, "0")}:00Z`,
            status: "completed",
          }),
        ]),
      ),
    ),
    {
      hasMore: true,
      onLoadMore,
    },
  );

  act(() => {
    vi.runAllTimers();
  });

  expect(onLoadMore).not.toHaveBeenCalled();
  vi.useRealTimers();
});
it("backfills immediately on first paint when the initial grid is not scrollable yet", () => {
  vi.useFakeTimers();
  vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(1700);
  vi.spyOn(window, "innerHeight", "get").mockReturnValue(900);
  vi.spyOn(document.documentElement, "scrollHeight", "get").mockReturnValue(640);
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
    this: HTMLElement,
  ) {
    if (this.getAttribute("data-testid") === "dashboard-working-conversations-grid") {
      return {
        x: 0,
        y: 0,
        top: 0,
        bottom: 640,
        left: 0,
        right: 1200,
        width: 1200,
        height: 640,
        toJSON: () => ({}),
      } satisfies DOMRect;
    }
    return {
      x: 0,
      y: 0,
      top: 0,
      bottom: 0,
      left: 0,
      right: 0,
      width: 0,
      height: 0,
      toJSON: () => ({}),
    } satisfies DOMRect;
  });
  const onLoadMore = vi.fn();

  renderSection(
    createResponse(
      Array.from({ length: 4 }, (_, index) =>
        createConversation(`pck-underflow-${index + 1}`, [
          createPreview({
            id: index + 1,
            invokeId: `invoke-underflow-${index + 1}`,
            occurredAt: `2026-04-04T10:${String(59 - index).padStart(2, "0")}:00Z`,
            status: "completed",
          }),
        ]),
      ),
    ),
    {
      hasMore: true,
      onLoadMore,
    },
  );

  act(() => {
    vi.runAllTimers();
  });

  expect(onLoadMore).toHaveBeenCalledTimes(1);
  vi.useRealTimers();
});
it("does not eagerly prefetch on mount when the section starts below the fold", () => {
  vi.useFakeTimers();
  vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(1700);
  vi.spyOn(window, "innerHeight", "get").mockReturnValue(900);
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
    this: HTMLElement,
  ) {
    if (this.getAttribute("data-testid") === "dashboard-working-conversations-grid") {
      return {
        x: 0,
        y: 1_120,
        top: 1_120,
        bottom: 1_760,
        left: 0,
        right: 1200,
        width: 1200,
        height: 640,
        toJSON: () => ({}),
      } satisfies DOMRect;
    }
    return {
      x: 0,
      y: 0,
      top: 0,
      bottom: 0,
      left: 0,
      right: 0,
      width: 0,
      height: 0,
      toJSON: () => ({}),
    } satisfies DOMRect;
  });
  const onLoadMore = vi.fn();

  renderSection(
    createResponse(
      Array.from({ length: 4 }, (_, index) =>
        createConversation(`pck-below-fold-${index + 1}`, [
          createPreview({
            id: index + 1,
            invokeId: `invoke-below-fold-${index + 1}`,
            occurredAt: `2026-04-04T10:${String(59 - index).padStart(2, "0")}:00Z`,
            status: "completed",
          }),
        ]),
      ),
    ),
    {
      hasMore: true,
      onLoadMore,
    },
  );

  act(() => {
    vi.runAllTimers();
  });

  expect(onLoadMore).not.toHaveBeenCalled();
  vi.useRealTimers();
});
it("continues initial load-more on mount when the page restores near the visible section bottom", () => {
  vi.useFakeTimers();
  vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(1700);
  vi.spyOn(window, "innerHeight", "get").mockReturnValue(900);
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
    this: HTMLElement,
  ) {
    if (this.getAttribute("data-testid") === "dashboard-working-conversations-grid") {
      return {
        x: 0,
        y: -260,
        top: -260,
        bottom: 1_160,
        left: 0,
        right: 1200,
        width: 1200,
        height: 1_420,
        toJSON: () => ({}),
      } satisfies DOMRect;
    }
    return {
      x: 0,
      y: 0,
      top: 0,
      bottom: 0,
      left: 0,
      right: 0,
      width: 0,
      height: 0,
      toJSON: () => ({}),
    } satisfies DOMRect;
  });
  const onLoadMore = vi.fn();

  renderSection(
    createResponse(
      Array.from({ length: 4 }, (_, index) =>
        createConversation(`pck-restored-${index + 1}`, [
          createPreview({
            id: index + 1,
            invokeId: `invoke-restored-${index + 1}`,
            occurredAt: `2026-04-04T10:${String(59 - index).padStart(2, "0")}:00Z`,
            status: "completed",
          }),
        ]),
      ),
    ),
    {
      hasMore: true,
      onLoadMore,
    },
  );

  act(() => {
    vi.runAllTimers();
  });

  expect(onLoadMore).toHaveBeenCalledTimes(1);
  vi.useRealTimers();
});
it("does not keep loading more after the section has scrolled above the viewport", () => {
  vi.useFakeTimers();
  vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(1700);
  vi.spyOn(window, "innerHeight", "get").mockReturnValue(900);
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
    this: HTMLElement,
  ) {
    if (this.getAttribute("data-testid") === "dashboard-working-conversations-grid") {
      return {
        x: 0,
        y: -1_320,
        top: -1_320,
        bottom: -40,
        left: 0,
        right: 1200,
        width: 1200,
        height: 1_280,
        toJSON: () => ({}),
      } satisfies DOMRect;
    }
    return {
      x: 0,
      y: 0,
      top: 0,
      bottom: 0,
      left: 0,
      right: 0,
      width: 0,
      height: 0,
      toJSON: () => ({}),
    } satisfies DOMRect;
  });
  const onLoadMore = vi.fn();

  renderSection(
    createResponse(
      Array.from({ length: 4 }, (_, index) =>
        createConversation(`pck-above-viewport-${index + 1}`, [
          createPreview({
            id: index + 1,
            invokeId: `invoke-above-viewport-${index + 1}`,
            occurredAt: `2026-04-04T10:${String(59 - index).padStart(2, "0")}:00Z`,
            status: "completed",
          }),
        ]),
      ),
    ),
    {
      hasMore: true,
      onLoadMore,
    },
  );

  act(() => {
    vi.runAllTimers();
    window.dispatchEvent(new Event("scroll"));
  });

  expect(onLoadMore).not.toHaveBeenCalled();
  vi.useRealTimers();
});
it("does not load more hidden conversations from the upstream-account tab", () => {
  vi.useFakeTimers();
  vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(1700);
  vi.spyOn(window, "innerHeight", "get").mockReturnValue(900);
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
    this: HTMLElement,
  ) {
    if (this.getAttribute("data-testid") === "dashboard-working-conversations-grid") {
      return {
        x: 0,
        y: 0,
        top: 0,
        bottom: 1_360,
        left: 0,
        right: 1200,
        width: 1200,
        height: 1_360,
        toJSON: () => ({}),
      } satisfies DOMRect;
    }
    if (this.getAttribute("data-testid") === "dashboard-upstream-account-grid") {
      return {
        x: 0,
        y: 0,
        top: 0,
        bottom: 640,
        left: 0,
        right: 1200,
        width: 1200,
        height: 640,
        toJSON: () => ({}),
      } satisfies DOMRect;
    }
    return {
      x: 0,
      y: 0,
      top: 0,
      bottom: 0,
      left: 0,
      right: 0,
      width: 0,
      height: 0,
      toJSON: () => ({}),
    } satisfies DOMRect;
  });
  upstreamAccountActivityMock.data = createUpstreamAccountActivityResponse();
  const onLoadMore = vi.fn();

  renderSection(
    createResponse(
      Array.from({ length: 4 }, (_, index) =>
        createConversation(`pck-hidden-${index + 1}`, [
          createPreview({
            id: index + 1,
            invokeId: `invoke-hidden-${index + 1}`,
            occurredAt: `2026-04-04T10:${String(59 - index).padStart(2, "0")}:00Z`,
            status: "completed",
          }),
        ]),
      ),
    ),
    {
      hasMore: true,
      onLoadMore,
    },
  );

  const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find((node) =>
    node.textContent?.includes("上游账号"),
  );
  if (!(accountTab instanceof HTMLButtonElement)) {
    throw new Error("missing upstream account tab");
  }

  act(() => {
    fireEvent.click(accountTab);
    vi.runOnlyPendingTimers();
    window.dispatchEvent(new Event("scroll"));
  });

  expect(onLoadMore).not.toHaveBeenCalled();
  vi.useRealTimers();
});
it("falls back to downstream-facing diagnostics in the dashboard card summary", () => {
  renderSection(
    createResponse([
      createConversation("pck-downstream-dashboard", [
        createPreview({
          id: 9,
          invokeId: "invoke-downstream-dashboard",
          occurredAt: "2026-04-04T10:04:00Z",
          status: "failed",
          failureClass: "client_abort",
          failureKind: "downstream_closed",
          downstreamStatusCode: 200,
          downstreamErrorMessage:
            "[downstream_closed] downstream closed while streaming upstream response",
        }),
      ]),
    ]),
  );

  const text = host?.textContent ?? "";
  expect(text).toContain("[downstream_closed] downstream closed while streaming upstream response");
});
it("renders warning success status labels in dashboard recent cards via the shared tooltip", async () => {
  renderSection(
    createResponse([
      createConversation("pck-warning-success", [
        createPreview({
          id: 10,
          invokeId: "invoke-warning-success-dashboard",
          occurredAt: "2026-04-04T10:05:00Z",
          status: "warning_success",
          failureClass: "none",
        }),
      ]),
    ]),
  );

  const statusNode = host?.querySelector(
    '[data-testid="dashboard-inline-invocation-status"]',
  ) as HTMLElement | null;
  expect(statusNode?.getAttribute("title")).toBeNull();
  expect(statusNode?.getAttribute("aria-label") ?? "").toContain("警告成功");

  await act(async () => {
    statusNode?.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
  });

  await waitFor(() => {
    const tooltip = Array.from(document.body.querySelectorAll('[role="tooltip"]')).find((node) =>
      node.textContent?.includes("警告成功"),
    );
    expect(tooltip).toBeInstanceOf(HTMLElement);
  });
});
it("keeps warning success recent rows icon-only in upstream-account activity and exposes details through the shared tooltip", async () => {
  const upstreamActivity = createUpstreamAccountActivityResponse();
  const recentInvocation = upstreamActivity.accounts[0]?.recentInvocations[0];
  if (!recentInvocation) throw new Error("missing recent invocation");
  const account = requireTestValue(upstreamActivity.accounts[0], "upstream account");
  account.recentInvocations[0] = {
    ...recentInvocation,
    status: "warning_success",
    failureKind: "downstream_closed",
    failureClass: "none",
  };
  upstreamAccountActivityMock.data = upstreamActivity;

  renderSection(
    createResponse([
      createConversation("pck-warning-success-account", [
        createPreview({
          id: 20,
          invokeId: "invoke-warning-success-account",
          occurredAt: "2026-04-04T10:05:00Z",
          status: "warning_success",
          failureClass: "none",
        }),
      ]),
    ]),
  );

  const accountTab = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find((node) =>
    node.textContent?.includes("上游账号"),
  );
  if (!(accountTab instanceof HTMLButtonElement)) {
    throw new Error("missing upstream account tab");
  }

  act(() => {
    fireEvent.click(accountTab);
  });

  const statusNode = host?.querySelector(
    '[data-testid="dashboard-inline-invocation-status"]',
  ) as HTMLElement | null;
  expect(statusNode?.textContent ?? "").not.toContain("警告成功");
  expect(statusNode?.getAttribute("title")).toBeNull();
  expect(statusNode?.getAttribute("aria-label") ?? "").toContain("警告成功");

  await act(async () => {
    statusNode?.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
  });

  await waitFor(() => {
    const tooltip = Array.from(document.body.querySelectorAll('[role="tooltip"]')).find((node) =>
      node.textContent?.includes("警告成功"),
    );
    expect(tooltip).toBeInstanceOf(HTMLElement);
  });
});
it("renders a fixed previous-invocation placeholder when a conversation has only one call", () => {
  renderSection(
    createResponse([
      createConversation("pck-single", [
        createPreview({
          id: 1,
          invokeId: "invoke-1",
          occurredAt: "2026-04-04T10:04:00Z",
          status: "completed",
        }),
      ]),
    ]),
  );

  const placeholders = host?.querySelectorAll(
    '[data-testid="dashboard-working-conversation-placeholder"]',
  );

  expect(placeholders).toHaveLength(2);
  expect(
    host?.querySelector(
      '[data-testid="dashboard-working-conversation-placeholder"][data-slot-kind="previous"]',
    )?.textContent,
  ).toContain("暂无上一条调用");
  expect(
    host?.querySelector(
      '[data-testid="dashboard-working-conversation-placeholder"][data-slot-kind="earlier"]',
    )?.textContent,
  ).toContain("暂无更早调用");
  for (const placeholder of placeholders ?? []) {
    expect(placeholder.textContent).toMatch(/暂无(上一条|更早)调用/);
    expect(placeholder.querySelectorAll(".working-conversation-placeholder-line")).toHaveLength(0);
    expect(
      placeholder.querySelector('[data-testid="dashboard-working-conversation-placeholder-label"]'),
    ).not.toBeNull();
    expect(placeholder.getAttribute("role")).toBe("group");
    expect(placeholder.getAttribute("aria-live")).toBeNull();
  }
});
