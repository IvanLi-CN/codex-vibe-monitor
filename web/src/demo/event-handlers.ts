import { sse } from "msw";
import type { SubscriptionTopicDescriptor } from "../lib/sse";
import { subscribeToDemoRealtime } from "./events";
import { demoSummary, handleDemoRequest } from "./handlers";
import { demoModel } from "./model";

const DEMO_SCHEMA_EPOCH = "demo-2026-07";
let demoCursor = 1;

function decodeBase64Url(value: string) {
  const normalized = value.replace(/-/g, "+").replace(/_/g, "/");
  const padding = normalized.length % 4 === 0 ? "" : "=".repeat(4 - (normalized.length % 4));
  return atob(`${normalized}${padding}`);
}

function parseRequestedTopics(request: Request): SubscriptionTopicDescriptor[] {
  const raw = new URL(request.url).searchParams.get("topics");
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

function descriptorKey(descriptor: SubscriptionTopicDescriptor) {
  return JSON.stringify({
    topic: descriptor.topic,
    params: descriptor.params ?? {},
  });
}

async function requestTopicPayload(requestUrl: string, path: string) {
  const response = await handleDemoRequest(new Request(new URL(path, requestUrl).toString()));
  return response.json();
}

async function resolveTopicPayload(descriptor: SubscriptionTopicDescriptor, requestUrl: string) {
  switch (descriptor.topic) {
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

export const eventHandlers = [
  sse(`${import.meta.env.BASE_URL}events`, async ({ request, client, finalize }) => {
    if (demoModel.snapshot.scene === "network-failure") {
      client.error();
      return;
    }

    const topics = parseRequestedTopics(request);
    if (topics.length === 0) {
      client.send({
        data: JSON.stringify({ type: "summary", window: "current", summary: demoSummary() }),
      });
    } else {
      for (const descriptor of topics) {
        const payload = await resolveTopicPayload(descriptor, request.url);
        if (payload == null) continue;
        client.send({
          data: JSON.stringify({
            type: "snapshot",
            topic: descriptor,
            topicKey: descriptorKey(descriptor),
            schemaEpoch: DEMO_SCHEMA_EPOCH,
            cursor: demoCursor,
            payload,
          }),
        });
        demoCursor += 1;
      }
    }

    const unsubscribe = subscribeToDemoRealtime((payload) =>
      client.send({ data: JSON.stringify(payload) }),
    );
    finalize(unsubscribe);
  }),
];
