import { describe, expect, it } from "vitest";
import { resolvePromptCacheInvocationOutcome } from "./conversationRequestPoint";

describe("resolvePromptCacheInvocationOutcome", () => {
  it("treats running rows with explicit failure metadata as failures", () => {
    expect(
      resolvePromptCacheInvocationOutcome({
        status: "running",
        failureClass: "service_failure",
        failureKind: undefined,
        errorMessage: "",
        downstreamErrorMessage: "",
      }),
    ).toBe("failure");
  });

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

  it("treats pending rows with failure-kind metadata as failures", () => {
    expect(
      resolvePromptCacheInvocationOutcome({
        status: "pending",
        failureClass: undefined,
        failureKind: "upstream_response_failed",
        errorMessage: "",
        downstreamErrorMessage: "",
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

  it("keeps status-only http failures marked as failures", () => {
    expect(
      resolvePromptCacheInvocationOutcome({
        status: "http_500",
        failureClass: "none",
        failureKind: undefined,
        errorMessage: "",
        downstreamErrorMessage: "",
      }),
    ).toBe("failure");
  });
});
