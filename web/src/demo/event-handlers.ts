import { sse } from "msw";
import { subscribeToDemoRealtime } from "./events";
import { demoSummary } from "./handlers";
import { demoModel } from "./model";
import {
  DEMO_SCHEMA_EPOCH,
  demoTopicDescriptorKey,
  parseDemoRequestedTopics,
  resolveDemoTopicPayload,
} from "./topic-payloads";

export {
  DEMO_SCHEMA_EPOCH,
  demoTopicDescriptorKey,
  parseDemoRequestedTopics,
  resolveDemoTopicPayload,
} from "./topic-payloads";

let demoCursor = 1;

export const eventHandlers = [
  sse(`${import.meta.env.BASE_URL}events`, async ({ request, client, finalize }) => {
    if (demoModel.snapshot.scene === "network-failure") {
      client.error();
      return;
    }

    const topics = parseDemoRequestedTopics(request.url);
    if (topics.length === 0) {
      client.send({
        data: JSON.stringify({ type: "summary", window: "current", summary: demoSummary() }),
      });
    } else {
      for (const descriptor of topics) {
        const payload = await resolveDemoTopicPayload(descriptor, request.url);
        if (payload == null) continue;
        client.send({
          data: JSON.stringify({
            type: "snapshot",
            topic: descriptor,
            topicKey: demoTopicDescriptorKey(descriptor),
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
