/** @vitest-environment jsdom */

import { expect, it } from "vitest";
import { areInvocationModelsEquivalent, resolveInvocationModelDisplay } from "../../lib/invocation";

it("prefers response model, then legacy model, then request model", () => {
  expect(
    resolveInvocationModelDisplay({
      model: "gpt-5.4",
      requestModel: "gpt-5.4",
      responseModel: "gpt-5.5",
    }).primaryValue,
  ).toBe("gpt-5.5");
  expect(
    resolveInvocationModelDisplay({
      model: "gpt-5.4",
      requestModel: "gpt-5.6",
      responseModel: undefined,
    }).primaryValue,
  ).toBe("gpt-5.4");
  expect(
    resolveInvocationModelDisplay({
      model: undefined,
      requestModel: "gpt-5.6",
      responseModel: undefined,
    }).primaryValue,
  ).toBe("gpt-5.6");
});
it("treats case-only and dated-alias differences as equivalent", () => {
  expect(areInvocationModelsEquivalent(" GPT-5.4 ", "gpt-5.4")).toBe(true);
  expect(areInvocationModelsEquivalent("gpt-5.4-2026-02-25", "gpt-5.4")).toBe(true);
  expect(
    resolveInvocationModelDisplay({
      requestModel: "gpt-5.4-2026-02-25",
      responseModel: "GPT-5.4",
    }).hasMismatch,
  ).toBe(false);
});
it("marks mismatches only when both request and response models are meaningfully different", () => {
  expect(
    resolveInvocationModelDisplay({
      requestModel: "gpt-5.4",
      responseModel: "gpt-5.5",
    }).hasMismatch,
  ).toBe(true);
  expect(
    resolveInvocationModelDisplay({
      requestModel: "gpt-5.4",
      responseModel: undefined,
    }).hasMismatch,
  ).toBe(false);
});
