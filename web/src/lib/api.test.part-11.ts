import { expect, it, vi } from "vitest";
import { updateOauthLoginSession } from "./api";

it("preserves syncApplied=false from stale session sync responses", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => {
      return new Response(
        JSON.stringify({
          loginId: "login-1",
          status: "pending",
          authUrl: "https://auth.openai.com/authorize?login=1",
          redirectUri: "http://localhost:1455/oauth/callback",
          expiresAt: "2026-03-13T10:00:00.000Z",
          updatedAt: "2026-03-13T10:01:00.000Z",
          accountId: null,
          error: null,
          syncApplied: false,
        }),
        { status: 200, headers: { "Content-Type": "application/json" } },
      );
    }) as typeof fetch,
  );

  const response = await updateOauthLoginSession("login-1", {
    displayName: "Fresh OAuth",
    tagIds: [],
    isMother: false,
  });

  expect(response.syncApplied).toBe(false);
  expect(response.updatedAt).toBe("2026-03-13T10:01:00.000Z");
});
