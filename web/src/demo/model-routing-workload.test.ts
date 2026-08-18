import { describe, expect, it } from "vitest";
import {
  createDemoModelRouteFixtures,
  DEMO_MODEL_ROUTE_FIXTURES,
  DEMO_ROUTE_COMBINATIONS,
  DEMO_ROUTING_WORKLOAD_VERSION,
} from "./model-routing-workload";

describe("model routing operational workload", () => {
  it("is a versioned, deterministic request ledger with exact account-model relationships", () => {
    const regenerated = createDemoModelRouteFixtures();

    expect(DEMO_ROUTING_WORKLOAD_VERSION).toBe("operational-routing-v2");
    expect(regenerated).toEqual(DEMO_MODEL_ROUTE_FIXTURES);
    expect(DEMO_MODEL_ROUTE_FIXTURES).toHaveLength(126);
    expect(new Set(DEMO_MODEL_ROUTE_FIXTURES.map((fixture) => fixture.invocationId)).size).toBe(
      DEMO_MODEL_ROUTE_FIXTURES.length,
    );
    expect(
      DEMO_MODEL_ROUTE_FIXTURES.every((fixture) =>
        DEMO_ROUTE_COMBINATIONS.some(
          (route) => route.accountId === fixture.accountId && route.model === fixture.model,
        ),
      ),
    ).toBe(true);
  });

  it("keeps a dense recent workload while preserving causal recovery, cooldown, and degradation flows", () => {
    const recent = DEMO_MODEL_ROUTE_FIXTURES.filter((fixture) => fixture.minutesAgo <= 60);

    expect(DEMO_MODEL_ROUTE_FIXTURES.filter((fixture) => fixture.minutesAgo <= 15)).toHaveLength(
      18,
    );
    expect(recent).toHaveLength(60);
    expect(DEMO_MODEL_ROUTE_FIXTURES.filter((fixture) => fixture.minutesAgo <= 360)).toHaveLength(
      96,
    );
    expect(DEMO_MODEL_ROUTE_FIXTURES.filter((fixture) => fixture.minutesAgo <= 1_440)).toHaveLength(
      126,
    );
    expect(DEMO_MODEL_ROUTE_FIXTURES.find((fixture) => fixture.flow === "recovered")).toMatchObject(
      {
        accountId: 102,
        model: "gpt-5.5",
        terminalStatus: "success",
      },
    );
    expect(
      DEMO_MODEL_ROUTE_FIXTURES.find((fixture) => fixture.flow === "cooling_down"),
    ).toMatchObject({
      accountId: 102,
      model: "gpt-5.4-mini",
      terminalStatus: "http_502",
    });
    expect(DEMO_MODEL_ROUTE_FIXTURES.find((fixture) => fixture.flow === "degraded")).toMatchObject({
      accountId: 115,
      model: "gpt-5.6-terra",
      terminalStatus: "http_502",
    });
  });
});
