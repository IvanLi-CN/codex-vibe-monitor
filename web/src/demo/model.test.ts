import { afterEach, describe, expect, it } from "vitest";
import type { DemoRealtimePayload } from "./events";
import { subscribeToDemoRealtime } from "./events";
import { demoModel } from "./model";

afterEach(() => {
  demoModel.setScene("operational");
  demoModel.reset();
});

describe("demoModel", () => {
  it("resets each scene to deterministic seed data", () => {
    demoModel.setScene("attention");
    demoModel.createAccount();
    expect(demoModel.snapshot.accounts).toHaveLength(16);

    demoModel.reset();

    expect(demoModel.snapshot.scene).toBe("attention");
    expect(demoModel.snapshot.accounts).toHaveLength(15);
    expect(demoModel.snapshot.accounts.map((account) => account.groupName)).toEqual(
      expect.arrayContaining(["production", "research", "standby", "edge", null]),
    );
    expect(demoModel.snapshot.actions).toEqual([]);
  });

  it("drops sensitive settings fields before retaining a simulated update", () => {
    demoModel.updateSettings("/api/settings/proxy", {
      enabledModels: ["gpt-5.6-sol"],
      apiKey: "user-secret-must-not-persist",
      nested: { accessToken: "user-token-must-not-persist" },
    });

    const serialized = JSON.stringify(demoModel.snapshot);
    expect(serialized).toContain("gpt-5.6-sol");
    expect(serialized).not.toContain("user-secret-must-not-persist");
    expect(serialized).not.toContain("user-token-must-not-persist");
  });

  it("publishes a deterministic records event for the Inspector action", () => {
    let received: DemoRealtimePayload | undefined;
    const unsubscribe = subscribeToDemoRealtime((payload) => {
      received = payload;
    });

    demoModel.injectLiveEvent();
    unsubscribe();

    expect(received?.type).toBe("records");
    expect(received?.records[0]?.invokeId).toBe("demo-live-event-9911");
    expect(demoModel.snapshot.actions[0]?.label).toBe("注入模拟实时事件");
  });
});
