/* Pure credential parsing and normalization for account-pool imports. */

function normalizeImportedOauthRequiredString(
  value: unknown,
  fieldName: string,
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  if (typeof value !== "string" || value.trim().length === 0) {
    return {
      ok: false as const,
      error: t("accountPool.upstreamAccounts.import.local.requiredField", {
        fieldName,
      }),
    };
  }
  return {
    ok: true as const,
    value: value.trim(),
  };
}

function decodeImportedOauthBase64UrlUtf8(input: string) {
  const normalized = input.replace(/-/g, "+").replace(/_/g, "/");
  const padded = normalized.padEnd(normalized.length + ((4 - (normalized.length % 4)) % 4), "=");
  const binary = atob(padded);
  const bytes = Uint8Array.from(binary, (char) => char.charCodeAt(0));
  return new TextDecoder().decode(bytes);
}

function encodeImportedOauthBase64UrlJson(value: Record<string, unknown>) {
  const bytes = new TextEncoder().encode(JSON.stringify(value));
  let binary = "";
  bytes.forEach((byte) => {
    binary += String.fromCharCode(byte);
  });
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/g, "");
}

function parseImportedOauthJwtPayload(
  token: string,
  tokenName: "access_token" | "id_token",
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  const parts = token.split(".");
  if (parts.length !== 3 || parts.some((part) => part.trim().length === 0)) {
    return {
      ok: false as const,
      error: t("accountPool.upstreamAccounts.import.local.invalidJwt", {
        tokenName,
      }),
    };
  }

  try {
    const decoded = decodeImportedOauthBase64UrlUtf8(parts[1]);
    const payload = JSON.parse(decoded) as Record<string, unknown>;
    return {
      ok: true as const,
      payload,
    };
  } catch {
    return {
      ok: false as const,
      error: t("accountPool.upstreamAccounts.import.local.invalidJwt", {
        tokenName,
      }),
    };
  }
}

function parseImportedOauthJwtExpiration(payload: Record<string, unknown>) {
  const exp = payload.exp;
  if (typeof exp !== "number" || !Number.isFinite(exp)) {
    return null;
  }
  return exp;
}

function parseImportedOauthJwtPayloadOptional(token: string | null | undefined) {
  const normalized = token?.trim();
  if (!normalized) return null;
  const parts = normalized.split(".");
  if (parts.length !== 3 || parts.some((part) => part.trim().length === 0)) {
    return null;
  }
  try {
    const decoded = decodeImportedOauthBase64UrlUtf8(parts[1]);
    const payload = JSON.parse(decoded);
    return payload && typeof payload === "object" && !Array.isArray(payload)
      ? (payload as Record<string, unknown>)
      : null;
  } catch {
    return null;
  }
}

function isImportedOauthRfc3339Timestamp(value: string) {
  const normalized = value.trim();
  if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/.test(normalized)) {
    return false;
  }
  return !Number.isNaN(Date.parse(normalized));
}

function extractImportedOauthJwtEmail(payload: Record<string, unknown>) {
  if (typeof payload.email === "string" && payload.email.trim().length > 0) {
    return payload.email.trim();
  }
  const profile = payload.profile;
  if (
    profile &&
    typeof profile === "object" &&
    typeof (profile as Record<string, unknown>).email === "string"
  ) {
    return ((profile as Record<string, unknown>).email as string).trim();
  }
  return null;
}

function extractImportedOauthJwtAuth(payload: Record<string, unknown>) {
  const namespaced = payload["https://api.openai.com/auth"];
  if (namespaced && typeof namespaced === "object") {
    return namespaced as Record<string, unknown>;
  }
  const auth = payload.auth;
  if (auth && typeof auth === "object") {
    return auth as Record<string, unknown>;
  }
  return null;
}

function extractImportedOauthJwtAccountId(payload: Record<string, unknown>) {
  const auth = extractImportedOauthJwtAuth(payload);
  if (typeof auth?.chatgpt_account_id !== "string") return null;
  return auth.chatgpt_account_id.trim();
}

function extractImportedOauthJwtUserId(payload: Record<string, unknown>) {
  const auth = extractImportedOauthJwtAuth(payload);
  const userId =
    typeof auth?.chatgpt_user_id === "string"
      ? auth.chatgpt_user_id
      : typeof auth?.user_id === "string"
        ? auth.user_id
        : null;
  return userId?.trim() || null;
}

export function buildImportedOauthMatchKeyFromValues(
  chatgptUserId: string | null | undefined,
  email: string | null | undefined,
  chatgptAccountId: string | null | undefined,
) {
  const normalizedUserId = chatgptUserId?.trim().toLocaleLowerCase();
  if (normalizedUserId) {
    return `user:${normalizedUserId}`;
  }
  const normalizedAccountId = chatgptAccountId?.trim().toLocaleLowerCase();
  if (normalizedAccountId) {
    return `account:${normalizedAccountId}`;
  }
  const normalizedEmail = email?.trim().toLocaleLowerCase();
  if (normalizedEmail) {
    return `email:${normalizedEmail}`;
  }
  return null;
}

type ImportedWebSessionSource = {
  value: Record<string, unknown>;
  path: string;
};

export type ConvertedImportedWebSessionCredential = {
  content: string;
  email: string;
  chatgptAccountId: string;
  chatgptUserId: string | null;
  matchKey: string;
  sourcePath: string;
};

export type ParsedImportedOauthCredentialCandidate = {
  normalizedContent: string;
  email: string;
  chatgptAccountId: string;
  chatgptUserId: string | null;
  matchKey: string;
  sourceLabel: string;
};

export type ParsedImportedOauthCredentialRejection = {
  sourceLabel: string;
  reason: string;
};

export function isImportedPlainObject(value: unknown): value is Record<string, unknown> {
  return Boolean(value && typeof value === "object" && !Array.isArray(value));
}

export function firstImportedNonEmptyString(...values: unknown[]) {
  for (const value of values) {
    if (typeof value === "string" && value.trim().length > 0) {
      return value.trim();
    }
  }
  return null;
}

function getImportedObject(value: unknown) {
  return isImportedPlainObject(value) ? value : null;
}

function normalizeImportedTimestamp(value: unknown) {
  if (typeof value === "number" && Number.isFinite(value)) {
    const date = new Date(value > 1e11 ? value : value * 1000);
    return Number.isNaN(date.getTime()) ? null : date.toISOString();
  }
  if (typeof value !== "string" || value.trim().length === 0) return null;
  const numeric = Number(value);
  if (Number.isFinite(numeric) && value.trim().match(/^\d+(?:\.\d+)?$/)) {
    const date = new Date(numeric > 1e11 ? numeric : numeric * 1000);
    return Number.isNaN(date.getTime()) ? null : date.toISOString();
  }
  const parsed = Date.parse(value);
  return Number.isNaN(parsed) ? null : new Date(parsed).toISOString();
}

function importedEpochSecondsFromTimestamp(value: string | null | undefined) {
  if (!value) return null;
  const parsed = Date.parse(value);
  if (Number.isNaN(parsed)) return null;
  return Math.floor(parsed / 1000);
}

function buildNormalizedImportedOauthContent({
  email,
  accountId,
  accessToken,
  refreshToken,
  idToken,
  tokenType,
  expired,
  planType,
  chatgptUserId,
}: {
  email: string;
  accountId: string;
  accessToken: string;
  refreshToken: string | null;
  idToken: string;
  tokenType: string | null;
  expired: string;
  planType: string | null;
  chatgptUserId: string | null;
}) {
  return JSON.stringify({
    type: "codex",
    email,
    account_id: accountId,
    chatgpt_account_id: accountId,
    chatgpt_user_id: chatgptUserId ?? undefined,
    plan_type: planType ?? undefined,
    chatgpt_plan_type: planType ?? undefined,
    access_token: accessToken,
    refresh_token: refreshToken ?? undefined,
    id_token: idToken,
    token_type: tokenType ?? undefined,
    expired,
  });
}

export function buildImportedOauthCandidateFromStandardRecord(
  record: Record<string, unknown>,
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  const errors: string[] = [];
  const fail = () => {
    const uniqueErrors = Array.from(new Set(errors));
    return {
      ok: false as const,
      error: uniqueErrors.join("\n"),
      errors: uniqueErrors,
    };
  };

  const emailResult = normalizeImportedOauthRequiredString(record.email, "email", t);
  if (!emailResult.ok) errors.push(emailResult.error);
  const accountIdResult = normalizeImportedOauthRequiredString(record.account_id, "account_id", t);
  if (!accountIdResult.ok) errors.push(accountIdResult.error);
  const accessTokenResult = normalizeImportedOauthRequiredString(
    record.access_token,
    "access_token",
    t,
  );
  if (!accessTokenResult.ok) errors.push(accessTokenResult.error);
  const idTokenResult = normalizeImportedOauthRequiredString(record.id_token, "id_token", t);
  if (!idTokenResult.ok) errors.push(idTokenResult.error);

  if (
    Object.hasOwn(record, "expired") &&
    record.expired != null &&
    typeof record.expired !== "string"
  ) {
    errors.push(t("accountPool.upstreamAccounts.import.local.invalidExpired"));
  }

  const idTokenPayload = idTokenResult.ok
    ? parseImportedOauthJwtPayload(idTokenResult.value, "id_token", t)
    : null;
  if (idTokenPayload && !idTokenPayload.ok) errors.push(idTokenPayload.error);

  const jwtEmail =
    idTokenPayload?.ok === true ? extractImportedOauthJwtEmail(idTokenPayload.payload) : null;
  if (emailResult.ok && jwtEmail && jwtEmail.toLowerCase() !== emailResult.value.toLowerCase()) {
    errors.push(t("accountPool.upstreamAccounts.import.local.emailMismatch"));
  }

  const jwtAccountId =
    idTokenPayload?.ok === true ? extractImportedOauthJwtAccountId(idTokenPayload.payload) : null;
  if (accountIdResult.ok && jwtAccountId && jwtAccountId !== accountIdResult.value) {
    errors.push(t("accountPool.upstreamAccounts.import.local.accountIdMismatch"));
  }

  const rawExpired = typeof record.expired === "string" ? record.expired.trim() : "";
  let normalizedExpired = rawExpired;
  if (rawExpired) {
    if (!isImportedOauthRfc3339Timestamp(rawExpired)) {
      errors.push(t("accountPool.upstreamAccounts.import.local.invalidExpired"));
    }
  } else {
    const idTokenExp =
      idTokenPayload?.ok === true ? parseImportedOauthJwtExpiration(idTokenPayload.payload) : null;
    const accessTokenPayload =
      idTokenPayload?.ok === true && idTokenExp == null && accessTokenResult.ok
        ? parseImportedOauthJwtPayload(accessTokenResult.value, "access_token", t)
        : null;
    if (accessTokenPayload && !accessTokenPayload.ok) {
      errors.push(accessTokenPayload.error);
    }
    const accessTokenExp =
      accessTokenPayload?.ok === true
        ? parseImportedOauthJwtExpiration(accessTokenPayload.payload)
        : null;
    const derivedExpired =
      normalizeImportedTimestamp(idTokenExp) ?? normalizeImportedTimestamp(accessTokenExp);
    if (!derivedExpired) {
      errors.push(t("accountPool.upstreamAccounts.import.local.missingExpiry"));
    } else {
      normalizedExpired = derivedExpired;
    }
  }

  if (errors.length > 0) {
    return fail();
  }

  if (
    !emailResult.ok ||
    !accountIdResult.ok ||
    !accessTokenResult.ok ||
    !idTokenResult.ok ||
    !normalizedExpired
  ) {
    return fail();
  }

  const email = emailResult.value;
  const accountId = accountIdResult.value;
  const accessToken = accessTokenResult.value;
  const idToken = idTokenResult.value;

  const chatgptUserId =
    idTokenPayload?.ok === true ? extractImportedOauthJwtUserId(idTokenPayload.payload) : null;
  const planType =
    firstImportedNonEmptyString(
      record.plan_type,
      record.chatgpt_plan_type,
      idTokenPayload?.ok === true
        ? extractImportedOauthJwtAuth(idTokenPayload.payload)?.chatgpt_plan_type
        : null,
    ) ?? null;
  const normalizedContent = buildNormalizedImportedOauthContent({
    email,
    accountId,
    accessToken,
    refreshToken: firstImportedNonEmptyString(record.refresh_token),
    idToken,
    tokenType: firstImportedNonEmptyString(record.token_type),
    expired: normalizedExpired,
    planType,
    chatgptUserId,
  });
  const matchKey = buildImportedOauthMatchKeyFromValues(chatgptUserId, email, accountId);
  if (!matchKey) {
    errors.push(
      t("accountPool.upstreamAccounts.import.local.requiredField", {
        fieldName: "account_id",
      }),
    );
    return fail();
  }

  return {
    ok: true as const,
    normalizedContent,
    email,
    chatgptAccountId: accountId,
    chatgptUserId,
    matchKey,
    sourceLabel: email,
  };
}

export function buildImportedOauthCandidateFromSub2apiAccount(
  account: Record<string, unknown>,
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  const credentials = getImportedObject(account.credentials);
  if (!credentials) {
    return {
      ok: false as const,
      error: t("accountPool.upstreamAccounts.import.local.unsupportedSub2apiAccount"),
      sourceLabel: firstImportedNonEmptyString(account.name, account.email) ?? "account",
    };
  }
  return {
    sourceLabel:
      firstImportedNonEmptyString(credentials.email, account.name, account.email) ?? "account",
    ...buildImportedOauthCandidateFromStandardRecord(
      {
        email: credentials.email,
        account_id:
          credentials.chatgpt_account_id ??
          credentials.account_id ??
          credentials.chatgptAccountId ??
          account.account_id,
        access_token: credentials.access_token ?? credentials.accessToken,
        refresh_token: credentials.refresh_token ?? credentials.refreshToken,
        id_token: credentials.id_token ?? credentials.idToken,
        expired: credentials.expires_at ?? credentials.expired ?? account.expired,
        token_type: credentials.token_type ?? credentials.tokenType,
        plan_type: credentials.plan_type ?? credentials.chatgpt_plan_type,
        chatgpt_user_id:
          credentials.chatgpt_user_id ?? credentials.user_id ?? credentials.chatgptUserId,
      },
      t,
    ),
  };
}

export function isSupportedSub2apiOauthAccount(record: Record<string, unknown>) {
  return (
    firstImportedNonEmptyString(record.platform)?.toLowerCase() === "openai" &&
    firstImportedNonEmptyString(record.type)?.toLowerCase() === "oauth"
  );
}

export function buildImportedOauthSub2apiAccountLabel(
  record: Record<string, unknown>,
  index: number,
) {
  const credentials = getImportedObject(record.credentials);
  return (
    firstImportedNonEmptyString(credentials?.email, record.name, record.email) ??
    `account ${index + 1}`
  );
}

export function collectImportedWebSessionLikeObjects(value: unknown): ImportedWebSessionSource[] {
  const found: ImportedWebSessionSource[] = [];
  const visited = new WeakSet<object>();

  const visit = (item: unknown, path: string) => {
    if (!isImportedPlainObject(item) && !Array.isArray(item)) return;
    if (typeof item === "object" && item !== null) {
      if (visited.has(item)) return;
      visited.add(item);
    }

    if (isImportedPlainObject(item)) {
      const token = firstImportedNonEmptyString(
        item.accessToken,
        item.access_token,
        getImportedObject(item.token)?.accessToken,
        getImportedObject(item.token)?.access_token,
        getImportedObject(item.credentials)?.accessToken,
        getImportedObject(item.credentials)?.access_token,
      );
      const providerData = getImportedObject(item.providerSpecificData);
      const hasIdentity =
        isImportedPlainObject(item.user) ||
        firstImportedNonEmptyString(
          item.email,
          item.name,
          getImportedObject(item.account)?.id,
          item.account_id,
          item.chatgptAccountId,
          providerData?.chatgptAccountId,
          providerData?.chatgpt_account_id,
          item.id,
        ) != null;
      if (token && hasIdentity) {
        found.push({ value: item, path });
        return;
      }

      for (const [key, child] of Object.entries(item)) {
        if (
          key === "accessToken" ||
          key === "access_token" ||
          key === "sessionToken" ||
          key === "session_token"
        ) {
          continue;
        }
        visit(child, `${path}.${key}`);
      }
      return;
    }

    item.forEach((child, index) => {
      visit(child, `${path}[${index}]`);
    });
  };

  visit(value, "$");
  return found;
}

function buildSyntheticImportedWebSessionIdToken({
  email,
  accountId,
  planType,
  userId,
  expiresAt,
}: {
  email: string;
  accountId: string;
  planType: string | null;
  userId: string | null;
  expiresAt: string | null;
}) {
  const now = Math.trunc(Date.now() / 1000);
  const authInfo: Record<string, string> = {
    chatgpt_account_id: accountId,
  };
  if (planType) authInfo.chatgpt_plan_type = planType;
  if (userId) {
    authInfo.chatgpt_user_id = userId;
    authInfo.user_id = userId;
  }
  const payload: Record<string, unknown> = {
    iat: now,
    exp: importedEpochSecondsFromTimestamp(expiresAt) ?? now + 90 * 24 * 60 * 60,
    email,
    auth: authInfo,
    "https://api.openai.com/auth": authInfo,
  };
  return `${encodeImportedOauthBase64UrlJson({
    alg: "none",
    typ: "JWT",
    cpa_synthetic: true,
  })}.${encodeImportedOauthBase64UrlJson(payload)}.signature`;
}

export function convertImportedWebSessionRecord(
  record: Record<string, unknown>,
  sourcePath: string,
  t: (key: string, values?: Record<string, string | number>) => string,
): ConvertedImportedWebSessionCredential {
  const token = getImportedObject(record.token);
  const credentials = getImportedObject(record.credentials);
  const providerData = getImportedObject(record.providerSpecificData);
  const user = getImportedObject(record.user);
  const account = getImportedObject(record.account);

  const accessToken = firstImportedNonEmptyString(
    record.accessToken,
    record.access_token,
    token?.accessToken,
    token?.access_token,
    credentials?.accessToken,
    credentials?.access_token,
  );
  if (!accessToken) {
    throw new Error(t("accountPool.upstreamAccounts.importSession.local.missingAccessToken"));
  }

  const accessPayload = parseImportedOauthJwtPayloadOptional(accessToken);
  const auth = accessPayload ? extractImportedOauthJwtAuth(accessPayload) : null;
  const profile = getImportedObject(accessPayload?.["https://api.openai.com/profile"]);

  const inputIdToken = firstImportedNonEmptyString(
    record.idToken,
    record.id_token,
    token?.idToken,
    token?.id_token,
    credentials?.idToken,
    credentials?.id_token,
  );
  const idPayload = parseImportedOauthJwtPayloadOptional(inputIdToken);
  const idAuth = idPayload ? extractImportedOauthJwtAuth(idPayload) : null;

  const email = firstImportedNonEmptyString(
    user?.email,
    record.email,
    credentials?.email,
    providerData?.email,
    profile?.email,
    idPayload?.email,
    accessPayload?.email,
  );
  if (!email) {
    throw new Error(
      t("accountPool.upstreamAccounts.import.local.requiredField", {
        fieldName: "user.email",
      }),
    );
  }

  const accountId = firstImportedNonEmptyString(
    account?.id,
    record.account_id,
    record.chatgptAccountId,
    providerData?.chatgptAccountId,
    providerData?.chatgpt_account_id,
    credentials?.chatgpt_account_id,
    auth?.chatgpt_account_id,
    idAuth?.chatgpt_account_id,
    record.provider === "codex" ? record.id : null,
  );
  if (!accountId) {
    throw new Error(
      t("accountPool.upstreamAccounts.import.local.requiredField", {
        fieldName: "account.id",
      }),
    );
  }

  const userId = firstImportedNonEmptyString(
    user?.id,
    record.user_id,
    record.chatgptUserId,
    providerData?.chatgptUserId,
    providerData?.chatgpt_user_id,
    auth?.chatgpt_user_id,
    auth?.user_id,
    idAuth?.chatgpt_user_id,
    idAuth?.user_id,
  );
  const planType = firstImportedNonEmptyString(
    account?.planType,
    account?.plan_type,
    record.planType,
    record.plan_type,
    providerData?.chatgptPlanType,
    providerData?.chatgpt_plan_type,
    credentials?.plan_type,
    auth?.chatgpt_plan_type,
    idAuth?.chatgpt_plan_type,
  );
  const expiresAt =
    (accessPayload
      ? normalizeImportedTimestamp(parseImportedOauthJwtExpiration(accessPayload))
      : null) ??
    normalizeImportedTimestamp(record.expires) ??
    normalizeImportedTimestamp(record.expiresAt) ??
    normalizeImportedTimestamp(record.expired) ??
    normalizeImportedTimestamp(record.expires_at);
  if (!expiresAt) {
    throw new Error(t("accountPool.upstreamAccounts.import.local.missingExpiry"));
  }

  const refreshToken = firstImportedNonEmptyString(
    record.refreshToken,
    record.refresh_token,
    token?.refreshToken,
    token?.refresh_token,
    credentials?.refreshToken,
    credentials?.refresh_token,
  );
  const sessionToken = firstImportedNonEmptyString(
    record.sessionToken,
    record.session_token,
    token?.sessionToken,
    token?.session_token,
    credentials?.sessionToken,
    credentials?.session_token,
  );
  const idToken =
    inputIdToken ??
    buildSyntheticImportedWebSessionIdToken({
      email,
      accountId,
      planType,
      userId,
      expiresAt,
    });

  const content = JSON.stringify({
    type: "codex",
    email,
    account_id: accountId,
    chatgpt_account_id: accountId,
    chatgpt_user_id: userId ?? undefined,
    plan_type: planType ?? undefined,
    chatgpt_plan_type: planType ?? undefined,
    access_token: accessToken,
    refresh_token: refreshToken ?? undefined,
    id_token: idToken,
    session_token: sessionToken ?? undefined,
    expired: expiresAt,
  });
  const matchKey = buildImportedOauthMatchKeyFromValues(userId, email, accountId);
  if (!matchKey) {
    throw new Error(
      t("accountPool.upstreamAccounts.import.local.requiredField", {
        fieldName: "account.id",
      }),
    );
  }
  return {
    content,
    email,
    chatgptAccountId: accountId,
    chatgptUserId: userId,
    matchKey,
    sourcePath,
  };
}
