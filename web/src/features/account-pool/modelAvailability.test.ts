import { describe, expect, it } from "vitest";
import type { ModelRoutingState, UpstreamAccountSummary } from "../../lib/api";
import { modelAvailability } from "./ModelRoutingHealthPanel";

const model = (state: string, probeRequired = false): ModelRoutingState => ({
  model: "model-a",
  state,
  lastFailureKind: "model_quota",
  probeRequired,
  priority: "normal",
  failureCount: 0,
  lastSeenAt: "unknown",
});

describe("account × model availability", () => {
  it("isolates a model cooldown from healthy siblings and unknown catalog models", () => {
    expect(
      [model("cooling_down"), model("available"), model("unknown")].map((route) =>
        modelAvailability(route),
      ),
    ).toEqual(["model_rate_limited", "available", "unknown"]);
  });
  it("shows recovery probes and degraded routes as verifying", () => {
    expect(modelAvailability(model("cooling_down", true))).toBe("verifying");
    expect(modelAvailability(model("degraded"))).toBe("verifying");
  });
  it("does not infer model limits from exhausted account snapshots or Reserve", () => {
    const account = {
      workStatus: "idle",
      primaryWindow: { usedPercent: 100 },
      reserve: { availability: "unavailable", limitReached: true },
    } as UpstreamAccountSummary;
    expect(modelAvailability(model("available"), account)).toBe("available");
    expect(modelAvailability(model("unknown"), account)).toBe("unknown");
    expect(modelAvailability(model("available"), { ...account, workStatus: "rate_limited" })).toBe(
      "account_rate_limited",
    );
  });
  it("uses current successful state even when historical failures remain", () => {
    expect(
      modelAvailability({ ...model("available"), lastFailureKind: "model_quota", failureCount: 5 }),
    ).toBe("available");
  });
});
