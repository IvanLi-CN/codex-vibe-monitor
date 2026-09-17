import { expect, it, vi } from "vitest";
import { createOauthMailboxSession } from "./api";

it("accepts unsupported responses even when emailAddress is blank", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => {
      return new Response(
        JSON.stringify({
          supported: false,
          emailAddress: "",
          reason: "invalid_format",
        }),
        { status: 200, headers: { "Content-Type": "application/json" } },
      );
    }) as typeof fetch,
  );

  await expect(createOauthMailboxSession({ emailAddress: "" })).resolves.toEqual({
    supported: false,
    emailAddress: "",
    reason: "invalid_format",
  });
});
