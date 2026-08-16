export type DemoModelRouteFlow = "available" | "recovered" | "degraded" | "cooling_down";

export type DemoModelRouteFixture = {
  invocationId: number;
  accountId: number;
  model: string;
  terminalStatus: "success" | "http_502";
  flow: DemoModelRouteFlow;
  minutesAgo: number;
};

type DemoRouteCombination = {
  accountId: number;
  model: string;
};

// This is a deterministic operational fixture, not independently random mock fields.
export const DEMO_ROUTING_WORKLOAD_VERSION = "operational-routing-v2";

export const DEMO_ROUTE_COMBINATIONS: DemoRouteCombination[] = [
  { accountId: 102, model: "gpt-5.5" },
  { accountId: 106, model: "gpt-5.5" },
  { accountId: 110, model: "gpt-5.5" },
  { accountId: 102, model: "gpt-5.4-mini" },
  { accountId: 108, model: "gpt-5.4-mini" },
  { accountId: 115, model: "gpt-5.4-mini" },
  { accountId: 106, model: "gpt-5.6-terra" },
  { accountId: 112, model: "gpt-5.6-terra" },
  { accountId: 115, model: "gpt-5.6-terra" },
];

const DEMO_ROUTE_SPECIAL_FLOWS: Record<number, DemoModelRouteFlow> = {
  0: "recovered",
  3: "cooling_down",
  8: "degraded",
};

function minutesAgoForRouteCall(index: number) {
  if (index < 18) return 1 + index * 0.75;
  if (index < 60) return 16 + (index - 18);
  if (index < 96) return 70 + (index - 60) * 8;
  return 390 + (index - 96) * 32;
}

function terminalStatusForFlow(flow: DemoModelRouteFlow) {
  return flow === "available" || flow === "recovered" ? "success" : "http_502";
}

export function createDemoModelRouteFixtures(): DemoModelRouteFixture[] {
  return Array.from({ length: 126 }, (_, index) => {
    const combination = DEMO_ROUTE_COMBINATIONS[index % DEMO_ROUTE_COMBINATIONS.length];
    const flow = DEMO_ROUTE_SPECIAL_FLOWS[index] ?? "available";
    return {
      invocationId: 10_000 + index,
      accountId: combination.accountId,
      model: combination.model,
      terminalStatus: terminalStatusForFlow(flow),
      flow,
      minutesAgo: minutesAgoForRouteCall(index),
    };
  });
}

export const DEMO_MODEL_ROUTE_FIXTURES = createDemoModelRouteFixtures();
