import { expect, it, vi } from "vitest";
import { fetchSystemTaskRuns } from "./api";

it("preserves the additive cursor contract", async () => {
  const requestedUrls: string[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL) => {
      requestedUrls.push(String(input));
      return new Response(
        JSON.stringify({
          items: [],
          total: 2,
          page: 1,
          pageSize: 1,
          nextCursor: "eyJzdGFydGVkQXQiOiIyMDI2LTA2LTIyVDA5OjE1OjAwWiIsImlkIjoyfQ",
        }),
        { status: 200, headers: { "Content-Type": "application/json" } },
      );
    }) as typeof fetch,
  );

  const response = await fetchSystemTaskRuns({
    cursor: "cursor-token",
    pageSize: 1,
  });

  expect(requestedUrls).toEqual(["/api/system/tasks?pageSize=1&cursor=cursor-token"]);
  expect(response.nextCursor).toBe("eyJzdGFydGVkQXQiOiIyMDI2LTA2LTIyVDA5OjE1OjAwWiIsImlkIjoyfQ");
});
