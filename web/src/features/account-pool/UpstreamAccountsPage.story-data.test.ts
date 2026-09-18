import { describe, expect, it, vi } from "vitest";
import { createLiveUsageBuilder } from "./UpstreamAccountsPage.story-data";

describe("createLiveUsageBuilder", () => {
  it("preserves the scenario-specific OAuth and API-key usage inputs", () => {
    const buildOauthUsage = vi.fn((primary: number, secondary: number) => ({
      kind: "oauth",
      primary,
      secondary,
    }));
    const buildApiKeyUsage = vi.fn((primary: number, secondary: number) => ({
      kind: "apiKey",
      primary,
      secondary,
    }));
    const buildUsage = createLiveUsageBuilder(buildOauthUsage, buildApiKeyUsage);

    expect(buildUsage("oauth_codex", 23, "syncing")).toEqual({
      kind: "oauth",
      primary: 39,
      secondary: 19,
    });
    expect(buildUsage("api_key_codex", 23, "unavailable")).toEqual({
      kind: "apiKey",
      primary: 103,
      secondary: 383,
    });
    expect(buildUsage("oauth_codex", 23, "rateLimited")).toEqual({
      kind: "oauth",
      primary: 87,
      secondary: 70,
    });
    expect(buildUsage("api_key_codex", 23, "healthy")).toEqual({
      kind: "apiKey",
      primary: 49,
      secondary: 143,
    });
  });
});
