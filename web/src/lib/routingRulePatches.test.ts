import { describe, expect, it } from "vitest";
import type { EffectiveRoutingRule } from "./api";
import { applyRoutingRulePatchToEffectiveRule } from "./routingRulePatches";
import { buildDefaultStatusChangeReasons } from "./upstreamAccountStatusChangeReasons";

const rule: EffectiveRoutingRule = {
  allowCutOut: true,
  allowCutIn: true,
  priorityTier: "normal",
  fastModeRewriteMode: "keep_original",
  concurrencyLimit: 0,
  upstream429RetryEnabled: false,
  upstream429MaxRetries: 0,
  availableModels: ["gpt-5.6-sol"],
  availableModelsMode: "allowlist",
  statusChangeReasons: { ...buildDefaultStatusChangeReasons(), upstream_http_401: true },
  sourceTagIds: [],
  sourceTagNames: [],
  fieldSources: {
    allowCutOut: "group",
    allowCutIn: "root",
    priorityTier: "root",
    fastModeRewriteMode: "root",
    concurrencyLimit: "root",
    upstream429Retry: "root",
    availableModels: "group",
    systemDeniedModels: "root",
  },
  timeouts: {
    responsesFirstByteTimeoutSecs: 120,
    compactFirstByteTimeoutSecs: 300,
    responsesStreamTimeoutSecs: 300,
    compactStreamTimeoutSecs: 300,
  },
  timeoutFieldSources: {
    responsesFirstByteTimeoutSecs: "group",
    compactFirstByteTimeoutSecs: "root",
    responsesStreamTimeoutSecs: "root",
    compactStreamTimeoutSecs: "root",
  },
};

describe("applyRoutingRulePatchToEffectiveRule", () => {
  it("merges nested policy values and marks optimistic sources", () => {
    const next = applyRoutingRulePatchToEffectiveRule(rule, {
      availableModels: ["gpt-5.6-sol", "gpt-5.6-terra"],
      statusChangeReasons: { upstream_http_401: false },
      timeouts: { responsesFirstByteTimeoutSecs: 90 },
    });

    expect(next.availableModels).toEqual(["gpt-5.6-sol", "gpt-5.6-terra"]);
    expect(next.fieldSources?.availableModels).toBe("account");
    expect(next.statusChangeReasons?.upstream_http_401).toBe(false);
    expect(next.statusChangeReasonFieldSources?.upstream_http_401).toBe("account");
    expect(next.timeouts?.responsesFirstByteTimeoutSecs).toBe(90);
    expect(next.timeoutFieldSources?.responsesFirstByteTimeoutSecs).toBe("account");
  });

  it("keeps inherited values visible while clearing an override", () => {
    const next = applyRoutingRulePatchToEffectiveRule(rule, {
      availableModels: null,
      timeouts: { responsesFirstByteTimeoutSecs: null },
    });

    expect(next.availableModels).toEqual(rule.availableModels);
    expect(next.fieldSources?.availableModels).toBe("root");
    expect(next.timeouts?.responsesFirstByteTimeoutSecs).toBe(120);
    expect(next.timeoutFieldSources?.responsesFirstByteTimeoutSecs).toBe("root");
  });
});
