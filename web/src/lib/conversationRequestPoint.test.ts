import { describe, expect, it } from "vitest";
import { resolvePromptCacheInvocationOutcome } from "./conversationRequestPoint";

describe("resolvePromptCacheInvocationOutcome", () => {
  it("treats completed live rows with error messages as failures", () => {
    expect(
      resolvePromptCacheInvocationOutcome({
        status: "completed",
        failureClass: undefined,
        failureKind: undefined,
        errorMessage: "upstream parse failed",
        downstreamErrorMessage: undefined,
      }),
    ).toBe("failure");
  });

  it("treats blank-status live rows with explicit error metadata as failures", () => {
    expect(
      resolvePromptCacheInvocationOutcome({
        status: "",
        failureClass: undefined,
        failureKind: undefined,
        errorMessage: "",
        downstreamErrorMessage: "pool upstream responded with 502",
      }),
    ).toBe("failure");
  });

  it("keeps blank-status rows neutral only when no failure metadata exists", () => {
    expect(
      resolvePromptCacheInvocationOutcome({
        status: "",
        failureClass: "none",
        failureKind: undefined,
        errorMessage: "",
        downstreamErrorMessage: "",
      }),
    ).toBe("neutral");
  });
});
