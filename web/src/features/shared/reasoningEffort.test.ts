import { describe, expect, it } from "vitest";
import {
  formatReasoningEffort,
  getReasoningEffortTone,
  REASONING_EFFORT_FALLBACK,
} from "./reasoningEffort";

describe("reasoning effort presentation", () => {
  it.each([
    ["none", "none"],
    [" minimal ", "minimal"],
    ["LOW", "low"],
    ["medium", "medium"],
    ["HIGH", "high"],
    [" xhigh ", "xhigh"],
    [" MAX ", "max"],
    ["ULTRA", "ultra"],
    ["Custom-Tier", "custom-tier"],
    [null, REASONING_EFFORT_FALLBACK],
    ["   ", REASONING_EFFORT_FALLBACK],
  ])("formats %j as %s", (value, expected) => {
    expect(formatReasoningEffort(value)).toBe(expected);
  });

  it.each([
    ["none", "none"],
    ["minimal", "minimal"],
    ["low", "low"],
    ["medium", "medium"],
    ["high", "high"],
    ["xhigh", "xhigh"],
    ["max", "max"],
    ["ultra", "ultra"],
    ["unknown-value", "unknown"],
    [null, "unknown"],
  ])("maps %j to the %s tone", (value, expected) => {
    expect(getReasoningEffortTone(value)).toBe(expected);
  });
});
