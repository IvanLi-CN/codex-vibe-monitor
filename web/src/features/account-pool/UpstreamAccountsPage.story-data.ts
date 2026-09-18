import type { LoginSessionStatusResponse, UpstreamAccountDetail } from "../../lib/api";

export const duplicateReasons = ["sharedChatgptAccountId", "sharedChatgptUserId"] as const;

type UsageBuilder = (primary: number, secondary: number) => object;

const usageInputs = {
  syncing: [
    [34, 18, 12, 16],
    [24, 28, 90, 120],
  ],
  unavailable: [
    [76, 12, 54, 18],
    [98, 18, 360, 80],
  ],
  rateLimited: [
    [84, 10, 61, 14],
    [108, 10, 410, 60],
  ],
  healthy: [
    [42, 18, 18, 14],
    [26, 30, 120, 140],
  ],
} as const;

export function createLiveUsageBuilder(
  buildOauthUsage: UsageBuilder,
  buildApiKeyUsage: UsageBuilder,
) {
  return (
    kind: UpstreamAccountDetail["kind"],
    seed: number,
    scenario: keyof typeof usageInputs,
  ) => {
    const [oauthInputs, apiKeyInputs] = usageInputs[scenario];
    const [primaryBase, primaryMod, secondaryBase, secondaryMod] =
      kind === "oauth_codex" ? oauthInputs : apiKeyInputs;
    const buildUsage = kind === "oauth_codex" ? buildOauthUsage : buildApiKeyUsage;
    return buildUsage(primaryBase + (seed % primaryMod), secondaryBase + (seed % secondaryMod));
  };
}

export function createPendingSession(loginId: string): LoginSessionStatusResponse {
  return {
    loginId,
    status: "pending",
    authUrl: `https://auth.openai.com/authorize?login_id=${loginId}`,
    redirectUri: "http://localhost:1455/auth/callback",
    expiresAt: "2027-03-11T13:30:00.000Z",
    accountId: null,
    error: null,
  };
}
