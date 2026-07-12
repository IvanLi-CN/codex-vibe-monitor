import { sse } from "msw";
import { subscribeToDemoRealtime } from "./events";
import { demoSummary } from "./handlers";
import { demoModel } from "./model";

export const eventHandlers = [
  sse(`${import.meta.env.BASE_URL}events`, ({ client }) => {
    if (demoModel.snapshot.scene === "network-failure") {
      client.error();
      return;
    }
    client.send({
      data: JSON.stringify({ type: "summary", window: "current", summary: demoSummary() }),
    });
    subscribeToDemoRealtime((payload) => client.send({ data: JSON.stringify(payload) }));
  }),
];
