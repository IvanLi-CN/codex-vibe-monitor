import { afterEach, describe, expect, it } from "vitest";
import type { DemoRealtimePayload } from "./events";
import { subscribeToDemoRealtime } from "./events";
import { demoModel } from "./model";

afterEach(() => {
  demoModel.setScene("operational");
  demoModel.reset();
});

describe("demoModel", () => {
  it("seeds official GPT-6 presets and pricing without exposing Terra", () => {
    expect(demoModel.snapshot.settings).toMatchObject({
      proxy: {
        models: expect.arrayContaining(["gpt-6-astra", "gpt-6-sol", "gpt-6-luna"]),
        enabledModels: expect.arrayContaining(["gpt-6-astra", "gpt-6-sol", "gpt-6-luna"]),
      },
      pricing: {
        catalogVersion: "openai-standard-2026-09-23",
        entries: expect.arrayContaining([
          expect.objectContaining({
            model: "gpt-6-astra",
            inputPer1m: 10,
            outputPer1m: 50,
            cacheReadPer1m: 1,
            cacheWritePer1m: 12.5,
            source: "official",
          }),
          expect.objectContaining({ model: "gpt-6-terra", source: "temporary" }),
        ]),
      },
    });

    const settings = demoModel.snapshot.settings as { proxy: { models: string[] } };
    expect(settings.proxy.models).not.toContain("gpt-6-terra");
  });

  it("resets each scene to deterministic seed data", () => {
    demoModel.setScene("attention");
    demoModel.createAccount();
    expect(demoModel.snapshot.accounts).toHaveLength(16);

    demoModel.reset();

    expect(demoModel.snapshot.scene).toBe("attention");
    expect(demoModel.snapshot.accounts).toHaveLength(15);
    expect(demoModel.snapshot.accounts.map((account) => account.groupName)).toEqual(
      expect.arrayContaining(["production", "research", "standby", "edge"]),
    );
    expect(
      demoModel.snapshot.accounts
        .filter((account) => account.kind === "api_key_codex")
        .every(
          (account) =>
            account.groupName === null &&
            Array.isArray(account.boundProxyKeys) &&
            account.boundProxyKeys.length > 0,
        ),
    ).toBe(true);
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
