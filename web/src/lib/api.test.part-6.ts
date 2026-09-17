import { expect, it, vi } from "vitest";
import { fetchUpstreamAccountAttempts, locateUpstreamAccountAttempt } from "./api";

it("uses distinct pagination and exact-location contracts", async () => {
  const requestedUrls: string[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL) => {
      requestedUrls.push(String(input));
      return new Response(JSON.stringify({ items: [], total: 0, page: 1, pageSize: 25 }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      });
    }) as typeof fetch,
  );

  await fetchUpstreamAccountAttempts(42, {
    type: "image",
    model: "gpt-image-1",
    stickyKey: "__unbound__",
    page: 3,
    pageSize: 25,
  });
  await locateUpstreamAccountAttempt(42, "4V7MYPJG", { pageSize: 25 });

  expect(requestedUrls).toEqual([
    "/api/pool/upstream-accounts/42/call-attempts?page=3&pageSize=25&type=image&model=gpt-image-1&stickyKey=__unbound__",
    "/api/pool/upstream-accounts/42/call-attempts/locate?attemptId=4V7MYPJG&pageSize=25",
  ]);
});
