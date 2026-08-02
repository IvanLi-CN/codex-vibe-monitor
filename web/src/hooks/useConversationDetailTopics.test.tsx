/** @vitest-environment jsdom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  resolveConversationDetailScope,
  useConversationDetailTopics,
} from "./useConversationDetailTopics";

const topicMocks = vi.hoisted(() => ({
  useSubscriptionTopic: vi.fn(),
}));

vi.mock("./useSubscriptionTopic", () => ({
  useSubscriptionTopic: topicMocks.useSubscriptionTopic,
}));

let host: HTMLDivElement | null = null;
let root: Root | null = null;

function TopicHarness(props: {
  activeTab: "overview" | "calls" | "settings" | "operations";
  scope: ReturnType<typeof resolveConversationDetailScope>;
}) {
  useConversationDetailTopics({
    open: true,
    activeTab: props.activeTab,
    scope: props.scope,
    operationsInfoType: "routing",
  });
  return null;
}

function renderHarness(props: Parameters<typeof TopicHarness>[0]) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(<TopicHarness {...props} />);
  });
}

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
  topicMocks.useSubscriptionTopic.mockReset();
});

describe("useConversationDetailTopics", () => {
  it("uses the prompt cache key scope unless a sticky account scope is explicit", () => {
    expect(resolveConversationDetailScope("pck-1")).toEqual({ promptCacheKey: "pck-1" });
    expect(
      resolveConversationDetailScope("pck-1", {
        promptCacheKey: "pck-1",
        stickyKey: "sticky-1",
        upstreamAccountId: 42,
      }),
    ).toEqual({ stickyKey: "sticky-1", upstreamAccountId: 42 });
    expect(resolveConversationDetailScope("sticky-1", { stickyKey: "sticky-1" })).toBeNull();
    expect(resolveConversationDetailScope(null)).toBeNull();
  });

  it("subscribes only to the visible tab with canonical sticky scope parameters", () => {
    topicMocks.useSubscriptionTopic.mockReturnValue({ data: null });
    renderHarness({
      activeTab: "calls",
      scope: { stickyKey: "sticky-1", upstreamAccountId: 42 },
    });

    expect(topicMocks.useSubscriptionTopic).toHaveBeenNthCalledWith(
      1,
      {
        topic: "invocation-history.window",
        params: { stickyKey: "sticky-1", upstreamAccountId: "42" },
      },
      true,
    );
    expect(topicMocks.useSubscriptionTopic).toHaveBeenNthCalledWith(
      2,
      {
        topic: "invocation-history.overview",
        params: { stickyKey: "sticky-1", upstreamAccountId: "42" },
      },
      false,
    );
    expect(topicMocks.useSubscriptionTopic).toHaveBeenNthCalledWith(
      3,
      {
        topic: "prompt-cache.conversation-binding.current",
        params: { stickyKey: "sticky-1", upstreamAccountId: "42" },
      },
      false,
    );
    expect(topicMocks.useSubscriptionTopic).toHaveBeenNthCalledWith(
      4,
      {
        topic: "prompt-cache.conversation-operations.window",
        params: { infoType: "routing", stickyKey: "sticky-1", upstreamAccountId: "42" },
      },
      false,
    );
  });
});
