/** @vitest-environment jsdom */

import { expect, it } from "vitest";
import {
  formatServiceTier,
  getFastIndicatorState,
  isPriorityServiceTier,
} from "../../lib/invocation";

it("normalizes and formats service tiers", () => {
  expect(formatServiceTier(" Priority ")).toBe("priority");
  expect(formatServiceTier("FLEX")).toBe("flex");
});
it("falls back to em dash for empty or missing service tiers", () => {
  expect(formatServiceTier(undefined)).toBe("—");
  expect(formatServiceTier("   ")).toBe("—");
});
it("treats only priority as fast mode", () => {
  expect(isPriorityServiceTier("priority")).toBe(true);
  expect(isPriorityServiceTier(" Priority ")).toBe(true);
  expect(isPriorityServiceTier("flex")).toBe(false);
  expect(isPriorityServiceTier(undefined)).toBe(false);
});
it("resolves fast indicator states from requested and billing tiers", () => {
  expect(getFastIndicatorState("priority", "priority", "priority")).toBe("effective");
  expect(getFastIndicatorState("priority", "default", "priority")).toBe("effective");
  expect(getFastIndicatorState("priority", "auto")).toBe("requested_only");
  expect(getFastIndicatorState("priority", undefined)).toBe("requested_only");
  expect(getFastIndicatorState("auto", "priority")).toBe("none");
  expect(getFastIndicatorState("flex", "auto")).toBe("none");
});
