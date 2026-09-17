/** @vitest-environment jsdom */

import { expect, it } from "vitest";
import { formatReasoningEffort, getReasoningEffortTone } from "./invocation-table-reasoning";

it("maps standard effort values onto the visual ladder", () => {
  expect(getReasoningEffortTone("none")).toBe("none");
  expect(getReasoningEffortTone(" minimal ")).toBe("minimal");
  expect(getReasoningEffortTone("LOW")).toBe("low");
  expect(getReasoningEffortTone("medium")).toBe("medium");
  expect(getReasoningEffortTone("high")).toBe("high");
  expect(getReasoningEffortTone("xhigh")).toBe("xhigh");
  expect(getReasoningEffortTone("max")).toBe("max");
  expect(getReasoningEffortTone("ultra")).toBe("ultra");
});
it("treats unknown raw strings as unknown tone", () => {
  expect(getReasoningEffortTone("custom-tier")).toBe("unknown");
  expect(getReasoningEffortTone("constructor")).toBe("unknown");
  expect(getReasoningEffortTone("__proto__")).toBe("unknown");
});
it("normalizes display values and preserves the shared missing fallback", () => {
  expect(formatReasoningEffort(" MAX ")).toBe("max");
  expect(formatReasoningEffort("ULTRA")).toBe("ultra");
  expect(formatReasoningEffort("CUSTOM-TIER")).toBe("custom-tier");
  expect(formatReasoningEffort(null)).toBe("—");
});
