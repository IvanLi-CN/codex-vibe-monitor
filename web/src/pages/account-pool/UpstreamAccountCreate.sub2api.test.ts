/** @vitest-environment jsdom */
import { describe, expect, it } from "vitest";
import {
  parseImportedOauthCredentialDocumentLocally,
  validateImportedOauthCredentialLocally,
} from "./UpstreamAccountCreate.shared";

const t = (key: string, values?: Record<string, string | number>) =>
  `${key}${values ? `:${JSON.stringify(values)}` : ""}`;
const expiry = "2030-01-01T00:00:00.000Z";
const jwt = (payload: Record<string, unknown>) =>
  `e30.${btoa(JSON.stringify(payload)).replace(/=/g, "").replace(/\+/g, "-").replace(/\//g, "_")}.sig`;
const account = (credentials: Record<string, unknown> = {}) => ({
  name: "Source account",
  platform: "openai",
  type: "oauth",
  credentials: {
    email: "member@example.com",
    chatgpt_account_id: "acct_shared",
    chatgpt_user_id: "user_member",
    access_token: "opaque-access-token",
    expires_at: expiry,
    ...credentials,
  },
});
const parse = (value: unknown) =>
  parseImportedOauthCredentialDocumentLocally(JSON.stringify(value), t);

describe("sub2api OAuth import compatibility", () => {
  it.each([
    undefined,
    "sub2api-data",
    "sub2api-bundle",
  ])("expands packages with type %s and reports unsupported members individually", (type) => {
    const result = parse({
      type,
      exported_at: "2026-10-19T00:00:00Z",
      proxies: [],
      accounts: [
        account(),
        { ...account(), type: "api_key" },
        { ...account(), platform: "claude" },
      ],
    });
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.candidates).toHaveLength(1);
    expect(result.rejected).toHaveLength(2);
    expect(result.candidates[0].matchKey).toBe("user:user_member");
    const content = JSON.parse(result.candidates[0].normalizedContent);
    expect(content).toMatchObject({ account_id: "acct_shared", expired: expiry });
    expect(content.refresh_token).toBeUndefined();
    expect(
      validateImportedOauthCredentialLocally(result.candidates[0].normalizedContent, t).ok,
    ).toBe(true);
  });

  it("derives identity and expiry from the access token when id_token is absent", () => {
    const result = parse(
      account({
        email: undefined,
        chatgpt_account_id: undefined,
        chatgpt_user_id: undefined,
        expires_at: undefined,
        access_token: jwt({
          exp: 1893456000,
          "https://api.openai.com/profile": { email: "from-token@example.com" },
          "https://api.openai.com/auth": {
            chatgpt_account_id: "acct_token",
            chatgpt_user_id: "user_token",
            chatgpt_plan_type: "team",
          },
        }),
      }),
    );
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.candidates[0]).toMatchObject({
      email: "from-token@example.com",
      chatgptAccountId: "acct_token",
      matchKey: "user:user_token",
    });
    expect(JSON.parse(result.candidates[0].normalizedContent)).toMatchObject({
      expired: expiry,
      plan_type: "team",
    });
  });

  it("derives account identity from the JWT account_id alias", () => {
    const result = parse(
      account({
        email: undefined,
        chatgpt_account_id: undefined,
        chatgpt_user_id: undefined,
        access_token: jwt({
          email: "from-token@example.com",
          auth: {
            account_id: "acct_alias",
            user_id: "user_alias",
          },
          exp: 1893456000,
        }),
      }),
    );
    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.candidates[0]).toMatchObject({
        email: "from-token@example.com",
        chatgptAccountId: "acct_alias",
        matchKey: "user:user_alias",
      });
    }
  });

  it("rejects token identity conflicts when an unrelated claim has the wrong type", () => {
    const result = parse(
      account({
        email: "different@example.com",
        access_token: jwt({
          email: "member@example.com",
          exp: 1893456000,
          "https://api.openai.com/auth": {
            chatgpt_account_id: "acct_shared",
            chatgpt_user_id: "user_member",
            chatgpt_plan_type: 123,
          },
        }),
      }),
    );
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.error).toContain("emailMismatch");
  });

  it("rejects conflicting identity claims within a token", () => {
    for (const payload of [
      {
        email: "member@example.com",
        "https://api.openai.com/profile": { email: "other@example.com" },
        "https://api.openai.com/auth": {
          chatgpt_account_id: "acct_shared",
          chatgpt_user_id: "user_member",
        },
        exp: 1893456000,
      },
      {
        email: "member@example.com",
        "https://api.openai.com/auth": {
          chatgpt_account_id: "acct_shared",
          chatgpt_user_id: "user_member",
        },
        auth: { user_id: "user_other" },
        exp: 1893456000,
      },
      {
        email: "member@example.com",
        "https://api.openai.com/auth": {
          chatgpt_account_id: "acct_shared",
          chatgpt_user_id: "user_member",
        },
        auth: { account_id: "acct_other" },
        exp: 1893456000,
      },
    ]) {
      const result = parse(account({ access_token: jwt(payload) }));
      expect(result.ok).toBe(false);
      if (!result.ok)
        expect(result.error).toMatch(/emailMismatch|accountIdMismatch|userIdMismatch/);
    }
  });

  it("rejects wrongly typed JWT identity claims", () => {
    for (const payload of [
      { email: 123, exp: 1893456000 },
      { "https://api.openai.com/auth": { chatgpt_account_id: 123 }, exp: 1893456000 },
      { "https://api.openai.com/auth": { user_id: 123 }, exp: 1893456000 },
    ]) {
      const result = parse(account({ access_token: jwt(payload) }));
      expect(result.ok).toBe(false);
      if (!result.ok)
        expect(result.error).toMatch(/emailMismatch|accountIdMismatch|userIdMismatch/);
    }
  });

  it("rejects wrongly typed explicit identity fields", () => {
    for (const [source, field, value, errorKey] of [
      ["credentials", "email", 123, "emailMismatch"],
      ["account", "email", [], "emailMismatch"],
      ["credentials", "chatgpt_account_id", {}, "accountIdMismatch"],
      ["account", "account_id", false, "accountIdMismatch"],
      ["credentials", "user_id", [], "userIdMismatch"],
      ["account", "chatgptUserId", 123, "userIdMismatch"],
    ] as const) {
      const input =
        source === "credentials" ? account({ [field]: value }) : { ...account(), [field]: value };
      const result = parse(input);
      expect(result.ok).toBe(false);
      if (!result.ok) expect(result.error).toContain(errorKey);
    }
  });

  it.each([
    1893456000,
    "1893456000",
    1893456000000,
    "1893456000000",
  ])("normalizes token expiry %s without using subscription expiry", (expires_at) => {
    const result = parse({ ...account({ expires_at }), expires_at: 1 });
    expect(result.ok).toBe(true);
    if (result.ok) expect(JSON.parse(result.candidates[0].normalizedContent).expired).toBe(expiry);
  });

  it("keeps members of a shared workspace distinct without ID tokens", () => {
    const result = parse({
      accounts: [account(), account({ email: "other@example.com", chatgpt_user_id: "user_other" })],
    });
    expect(result.ok).toBe(true);
    if (result.ok)
      expect(result.candidates.map((candidate) => candidate.matchKey)).toEqual([
        "user:user_member",
        "user:user_other",
      ]);
  });

  it.each([
    ["email", "another@example.com"],
    ["chatgpt_account_id", "acct_other"],
    ["chatgpt_user_id", "user_other"],
  ])("rejects conflicting %s instead of replacing the token identity", (field, value) => {
    const result = parse(
      account({
        [field]: value,
        access_token: jwt({
          email: "member@example.com",
          "https://api.openai.com/auth": {
            chatgpt_account_id: "acct_shared",
            chatgpt_user_id: "user_member",
          },
        }),
      }),
    );
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.error).toContain("Mismatch");
  });

  it.each([
    ["email", "email", "emailMismatch"],
    ["account_id", "account_id", "accountIdMismatch"],
    ["chatgpt_user_id", "chatgpt_user_id", "userIdMismatch"],
  ])("rejects conflicting duplicate explicit %s fields", (field, recordKey, errorKey) => {
    const credentialValues =
      field === "account_id"
        ? { chatgpt_account_id: "credential-value", account_id: "credential-value" }
        : { [field]: "credential-value" };
    const result = parse({
      ...account(credentialValues),
      [recordKey]: "record-value",
    });
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.error).toContain(errorKey);
  });

  it.each([
    ["chatgpt_account_id", "account_id", "accountIdMismatch"],
    ["chatgpt_user_id", "user_id", "userIdMismatch"],
  ])("rejects conflicting identity aliases within credentials", (firstKey, secondKey, errorKey) => {
    const result = parse(
      account({
        [firstKey]: "first-value",
        [secondKey]: "second-value",
      }),
    );
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.error).toContain(errorKey);
  });

  it("preserves identity supplied only on the outer sub2api account", () => {
    const result = parse({
      ...account({
        email: undefined,
        chatgpt_account_id: undefined,
        chatgpt_user_id: undefined,
      }),
      email: "outer@example.com",
      chatgpt_account_id: "acct_outer",
      chatgpt_user_id: "user_outer",
    });
    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.candidates[0]).toMatchObject({
        email: "outer@example.com",
        chatgptAccountId: "acct_outer",
        chatgptUserId: "user_outer",
        matchKey: "user:user_outer",
      });
    }
  });

  it("reports missing identity or expiry without requiring a fabricated ID token", () => {
    const result = parse(account({ email: undefined, expires_at: undefined }));
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.error).toContain("email");
      expect(result.error).toContain("missingExpiry");
      expect(result.error).not.toContain("id_token");
    }
  });

  it("does not hide an invalid explicit expiry or an invalid supplied ID token", () => {
    for (const credentials of [
      { expires_at: "not-a-date" },
      { expires_at: "January 1, 2030" },
      { id_token: "not-a-jwt" },
      { id_token: 123 },
      {
        id_token: `${jwt({
          email: "member@example.com",
          "https://api.openai.com/auth": {
            chatgpt_account_id: "acct_shared",
            chatgpt_user_id: "user_member",
          },
          exp: 1893456000,
        })}.trailing`,
      },
    ]) {
      expect(parse(account(credentials)).ok).toBe(false);
    }
  });

  it("preserves supplied ID tokens and explicit member metadata absent from their claims", () => {
    const idToken = jwt({ email: "member@example.com", exp: 1893457000 });
    const result = parse(account({ id_token: idToken, plan_type: "k12" }));
    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.candidates[0].matchKey).toBe("user:user_member");
      expect(JSON.parse(result.candidates[0].normalizedContent)).toMatchObject({
        id_token: idToken,
        plan_type: "k12",
      });
    }
  });

  it.each([
    { accounts: [] },
    { type: "sub2api-data", accounts: {} },
    { type: "unknown", accounts: [account()] },
    { type: 123, accounts: [account()] },
  ])("reports empty or malformed export packages directly", (value) => {
    const result = parse(value);
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.error).not.toContain("requiredField");
  });
});
