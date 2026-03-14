import type { LoginSessionStatusResponse } from '../lib/api'

export const duplicateReasons = ['sharedChatgptAccountId', 'sharedChatgptUserId'] as const

export function createPendingSession(loginId: string): LoginSessionStatusResponse {
  return {
    loginId,
    status: 'pending',
    authUrl: `https://auth.openai.com/authorize?login_id=${loginId}`,
    redirectUri: 'http://localhost:1455/auth/callback',
    expiresAt: '2026-03-11T13:30:00.000Z',
    accountId: null,
    error: null,
  }
}
