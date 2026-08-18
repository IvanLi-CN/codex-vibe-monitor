import type { SubscriptionTopicDescriptor } from "../lib/sse";
import { handleDemoRequest } from "./handlers";

export const DEMO_SCHEMA_EPOCH = "demo-2026-07";

function decodeBase64Url(value: string) {
  const normalized = value.replace(/-/g, "+").replace(/_/g, "/");
  const padding = normalized.length % 4 === 0 ? "" : "=".repeat(4 - (normalized.length % 4));
  return atob(`${normalized}${padding}`);
}

export function parseDemoRequestedTopics(requestUrl: string): SubscriptionTopicDescriptor[] {
  const raw = new URL(requestUrl).searchParams.get("topics");
  if (!raw) return [];
  try {
    const parsed = JSON.parse(decodeBase64Url(raw)) as unknown;
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((entry): entry is SubscriptionTopicDescriptor => {
      if (!entry || typeof entry !== "object") return false;
      const topic = (entry as { topic?: unknown }).topic;
      return typeof topic === "string" && topic.trim().length > 0;
    });
  } catch {
    return [];
  }
}

export function demoTopicDescriptorKey(descriptor: SubscriptionTopicDescriptor) {
  return JSON.stringify({
    topic: descriptor.topic,
    params: descriptor.params ?? {},
  });
}

async function requestTopicPayload(requestUrl: string, path: string) {
  const response = await handleDemoRequest(new Request(new URL(path, requestUrl).toString()));
  return response.json();
}

function topicSearchParams(descriptor: SubscriptionTopicDescriptor) {
  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(descriptor.params ?? {})) {
    if (value != null) search.set(key, `${value}`);
  }
  return search;
}

export async function resolveDemoTopicPayload(
  descriptor: SubscriptionTopicDescriptor,
  requestUrl: string,
) {
  const conversationScope = () => {
    const search = new URLSearchParams();
    if (descriptor.params?.promptCacheKey != null) {
      search.set("promptCacheKey", `${descriptor.params.promptCacheKey}`);
    }
    if (descriptor.params?.stickyKey != null) {
      search.set("stickyKey", `${descriptor.params.stickyKey}`);
    }
    if (descriptor.params?.upstreamAccountId != null) {
      search.set("upstreamAccountId", `${descriptor.params.upstreamAccountId}`);
    }
    return search;
  };
  switch (descriptor.topic) {
    case "stats.summary.current": {
      const search = topicSearchParams(descriptor);
      return requestTopicPayload(requestUrl, `/api/stats?${search.toString()}`);
    }
    case "forward-proxy.live":
      return requestTopicPayload(requestUrl, "/api/stats/forward-proxy");
    case "pool.model-routing-live": {
      const search = topicSearchParams(descriptor);
      return requestTopicPayload(requestUrl, `/api/pool/model-routing-live?${search.toString()}`);
    }
    case "prompt-cache.window": {
      const search = topicSearchParams(descriptor);
      return requestTopicPayload(
        requestUrl,
        `/api/stats/prompt-cache-conversations?${search.toString()}`,
      );
    }
    case "invocations.window": {
      const search = topicSearchParams(descriptor);
      return requestTopicPayload(requestUrl, `/api/invocations?${search.toString()}`);
    }
    case "stats.parallel-work.current": {
      const search = topicSearchParams(descriptor);
      return requestTopicPayload(requestUrl, `/api/stats/parallel-work?${search.toString()}`);
    }
    case "invocation-history.window": {
      const search = conversationScope();
      search.set("page", "1");
      search.set("pageSize", "50");
      search.set("sortBy", "occurredAt");
      search.set("sortOrder", "desc");
      return requestTopicPayload(requestUrl, `/api/invocations?${search.toString()}`);
    }
    case "invocation-history.overview": {
      const search = conversationScope();
      const [summary, window] = await Promise.all([
        requestTopicPayload(requestUrl, `/api/invocations/summary?${search.toString()}`),
        requestTopicPayload(requestUrl, `/api/invocations?${search.toString()}`),
      ]);
      const invocationWindow = window as { records?: unknown[]; total?: number };
      return {
        summary,
        records: invocationWindow.records ?? [],
        chartTotal: invocationWindow.total ?? 0,
        chartIsSampled: false,
      };
    }
    case "prompt-cache.conversation-binding.current": {
      const promptCacheKey = `${
        descriptor.params?.promptCacheKey ?? descriptor.params?.stickyKey ?? ""
      }`;
      return requestTopicPayload(
        requestUrl,
        `/api/stats/prompt-cache-conversation-bindings/${encodeURIComponent(promptCacheKey)}`,
      );
    }
    case "prompt-cache.conversation-operations.window": {
      const promptCacheKey = `${
        descriptor.params?.promptCacheKey ?? descriptor.params?.stickyKey ?? ""
      }`;
      const search = new URLSearchParams({ page: "1", pageSize: "20" });
      if (descriptor.params?.infoType != null) {
        search.set("infoType", `${descriptor.params.infoType}`);
      }
      return requestTopicPayload(
        requestUrl,
        `/api/stats/prompt-cache-conversation-binding-events/${encodeURIComponent(promptCacheKey)}?${search.toString()}`,
      );
    }
    case "dashboard.activity.current": {
      const search = new URLSearchParams();
      search.set("range", `${descriptor.params?.range ?? "today"}`);
      search.set("timeZone", `${descriptor.params?.timeZone ?? "UTC"}`);
      search.set("includeAccounts", `${descriptor.params?.includeAccounts ?? "true"}`);
      search.set("includeRecent", `${descriptor.params?.includeRecent ?? "true"}`);
      if (descriptor.params?.recentLimit != null) {
        search.set("recentLimit", `${descriptor.params.recentLimit}`);
      }
      return requestTopicPayload(requestUrl, `/api/stats/dashboard-activity?${search.toString()}`);
    }
    case "dashboard.working-conversations.current": {
      const search = new URLSearchParams();
      search.set("activityMinutes", "5");
      search.set("pageSize", `${descriptor.params?.pageSize ?? 20}`);
      search.set("recentInvocationLimit", `${descriptor.params?.recentInvocationLimit ?? 16}`);
      return requestTopicPayload(
        requestUrl,
        `/api/stats/prompt-cache-conversations?${search.toString()}`,
      );
    }
    case "stats.timeseries.open-window": {
      const search = new URLSearchParams();
      search.set("range", `${descriptor.params?.range ?? "today"}`);
      search.set("timeZone", `${descriptor.params?.timeZone ?? "UTC"}`);
      if (descriptor.params?.bucket != null) {
        search.set("bucket", `${descriptor.params.bucket}`);
      }
      if (descriptor.params?.settlementHour != null) {
        search.set("settlementHour", `${descriptor.params.settlementHour}`);
      }
      if (descriptor.params?.upstreamAccountId != null) {
        search.set("upstreamAccountId", `${descriptor.params.upstreamAccountId}`);
      }
      return requestTopicPayload(requestUrl, `/api/stats/timeseries?${search.toString()}`);
    }
    case "app.version":
      return requestTopicPayload(requestUrl, "/api/version");
    default:
      return null;
  }
}
