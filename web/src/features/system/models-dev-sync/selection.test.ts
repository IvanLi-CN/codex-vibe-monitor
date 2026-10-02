import { describe, expect, it } from "vitest";
import type {
  ModelsDevPriceCandidate,
  ModelsDevSyncMemoryState,
  PricingEntry,
} from "../../../lib/api";
import {
  applySelectionChanges,
  buildModelCandidateGroups,
  createApplicablePriceEntries,
  modelProviderKey,
  priceDifferenceFields,
  resolveModelCandidate,
  selectionMemoryByKey,
} from "./selection";

function candidate(
  model: string,
  providerId: string,
  overrides: Partial<ModelsDevPriceCandidate> = {},
): ModelsDevPriceCandidate {
  return {
    model,
    name: model,
    providerId,
    providerName: providerId,
    docUrl: null,
    status: null,
    inputPer1m: 1,
    outputPer1m: 2,
    cacheReadPer1m: null,
    cacheWritePer1m: null,
    reasoningPer1m: null,
    unsupportedDimensions: [],
    importable: true,
    ...overrides,
  };
}

function memory(
  modelSelections: ModelsDevSyncMemoryState["modelSelections"],
): ModelsDevSyncMemoryState {
  return {
    catalogBaselineInitialized: true,
    providerSelectionInitialized: true,
    providerSelections: [],
    modelSelections,
    quoteProviderChoices: [],
    unviewedModelIds: [],
  };
}

describe("models.dev price selection", () => {
  it("filters deprecated quotes before grouping while preserving unknown statuses", () => {
    const candidates = [
      candidate("same-model", "old", { status: "deprecated" }),
      candidate("same-model", "active", { status: "alpha" }),
      candidate("missing-status", "new", { status: null }),
    ];
    const groups = buildModelCandidateGroups(candidates, new Set(["old", "active", "new"]), false);

    expect(
      groups
        .find((group) => group.model === "same-model")
        ?.candidates.map((item) => item.providerId),
    ).toEqual(["active"]);
    expect(groups.some((group) => group.model === "missing-status")).toBe(true);
  });

  it("does not replace an unavailable remembered quote with a sole alternative", () => {
    const group = buildModelCandidateGroups(
      [candidate("shared", "alternative")],
      new Set(["alternative"]),
      false,
    )[0];

    expect(resolveModelCandidate(group, "temporarily-hidden")).toBeUndefined();
  });

  it("defaults to unchecked and keys remembered choices by model and provider", () => {
    const model = "model-a";
    const a = candidate(model, "provider-a");
    const b = candidate(model, "provider-b");
    const groups = buildModelCandidateGroups([a, b], new Set(["provider-a", "provider-b"]), false);
    const selection = selectionMemoryByKey(
      memory([
        { model, providerId: "provider-a", selected: false },
        { model, providerId: "provider-b", selected: true },
      ]),
    );

    expect(selection[modelProviderKey(model, "provider-a")]).toBe(false);
    expect(selection[modelProviderKey(model, "provider-b")]).toBe(true);
    expect(selection[modelProviderKey("another-model", "provider-b")]).toBeUndefined();
    expect(
      createApplicablePriceEntries(groups, new Map(), selection, {
        [model]: "provider-b",
      }),
    ).toHaveLength(1);
  });

  it("applies select, invert, and clear only to the supplied matching rows", () => {
    const initial = {
      [modelProviderKey("visible", "provider-a")]: true,
      [modelProviderKey("filtered", "provider-a")]: true,
    };
    const rows = [{ model: "visible", providerId: "provider-a" }];

    expect(applySelectionChanges(initial, rows, "invert")).toMatchObject({
      [modelProviderKey("visible", "provider-a")]: false,
      [modelProviderKey("filtered", "provider-a")]: true,
    });
    expect(
      applySelectionChanges(initial, rows, false)[modelProviderKey("visible", "provider-a")],
    ).toBe(false);
  });

  it("distinguishes missing prices from zero and omits unchanged selected prices", () => {
    const model = "model-a";
    const source = candidate(model, "provider-a", { cacheReadPer1m: 0 });
    const local: PricingEntry = {
      model,
      inputPer1m: 1,
      outputPer1m: 2,
      cacheInputPer1m: null,
      cacheReadPer1m: null,
      cacheWritePer1m: null,
      reasoningPer1m: null,
      source: "custom",
    };
    const differences = priceDifferenceFields(local, source);
    expect(differences).toEqual(new Set(["cacheReadPer1m"]));

    const noChange = candidate(model, "provider-a", { cacheReadPer1m: null });
    const selection = { [modelProviderKey(model, "provider-a")]: true };
    expect(
      createApplicablePriceEntries(
        buildModelCandidateGroups([noChange], new Set(["provider-a"]), false),
        new Map([[model, local]]),
        selection,
        {},
      ),
    ).toHaveLength(0);
  });

  it.each([
    { field: "inputPer1m", localValue: 1, incomingValue: 0.5 },
    { field: "inputPer1m", localValue: 1, incomingValue: null },
    { field: "outputPer1m", localValue: 2, incomingValue: 3 },
    { field: "outputPer1m", localValue: 2, incomingValue: null },
    { field: "cacheReadPer1m", localValue: null, incomingValue: 0 },
    { field: "cacheReadPer1m", localValue: 0, incomingValue: null },
    { field: "cacheWritePer1m", localValue: null, incomingValue: 0.25 },
    { field: "cacheWritePer1m", localValue: 0.25, incomingValue: null },
    { field: "reasoningPer1m", localValue: null, incomingValue: 0.4 },
    { field: "reasoningPer1m", localValue: 0.4, incomingValue: null },
  ] as const)("reports only a $field difference", ({ field, localValue, incomingValue }) => {
    const model = "model-a";
    const local: PricingEntry = {
      model,
      inputPer1m: 1,
      outputPer1m: 2,
      cacheInputPer1m: null,
      cacheReadPer1m: null,
      cacheWritePer1m: null,
      reasoningPer1m: null,
      source: "custom",
      [field]: localValue,
    };

    expect(
      priceDifferenceFields(local, candidate(model, "provider-a", { [field]: incomingValue })),
    ).toEqual(new Set([field]));
  });
});
