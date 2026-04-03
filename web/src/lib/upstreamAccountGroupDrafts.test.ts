import { describe, expect, it } from "vitest";
import { resolvePersistedGroupNodeShuntEnabled } from "./upstreamAccountGroupDrafts";

describe("resolvePersistedGroupNodeShuntEnabled", () => {
  it("preserves the current node shunt state when no draft toggle was edited", () => {
    expect(resolvePersistedGroupNodeShuntEnabled(false, undefined, true)).toBe(
      true,
    );
    expect(resolvePersistedGroupNodeShuntEnabled(false, undefined, false)).toBe(
      false,
    );
  });

  it("uses the draft toggle value when the user edited node shunt explicitly", () => {
    expect(resolvePersistedGroupNodeShuntEnabled(true, true, false)).toBe(true);
    expect(resolvePersistedGroupNodeShuntEnabled(true, false, true)).toBe(
      false,
    );
  });
});
