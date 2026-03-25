/* eslint-disable react-refresh/only-export-components */
import type { JSX } from 'react'
import { SystemNotificationProvider } from './ui/system-notifications'
import { I18nProvider } from '../i18n'
import UpstreamAccountCreatePage from '../pages/account-pool/UpstreamAccountCreate'
import type { LoginSessionStatusResponse, OauthMailboxSessionSupported, OauthMailboxStatus } from '../lib/api'
import {
  AccountPoolStoryRouter,
  StorybookUpstreamAccountsMock,
} from './UpstreamAccountsPage.story-helpers'

export { UpstreamAccountCreatePage }
export const upstreamAccountCreateMetaBase = {
  component: UpstreamAccountCreatePage,
  tags: ['autodocs'],
  parameters: {
    layout: 'fullscreen',
  },
  decorators: [
    (Story: () => JSX.Element) => (
      <I18nProvider>
        <SystemNotificationProvider>
          <StorybookUpstreamAccountsMock>
            <Story />
          </StorybookUpstreamAccountsMock>
        </SystemNotificationProvider>
      </I18nProvider>
    ),
  ],
}

export function createCompletedSession(loginId: string, accountId: number): LoginSessionStatusResponse {
  return {
    loginId,
    status: 'completed',
    authUrl: null,
    redirectUri: null,
    expiresAt: '2027-03-11T13:30:00.000Z',
    accountId,
    error: null,
  }
}

export function createMailboxSession(sessionId: string, emailAddress: string): OauthMailboxSessionSupported {
  return {
    supported: true,
    sessionId,
    emailAddress,
    expiresAt: '2027-03-20T12:50:00.000Z',
    source: 'generated',
  }
}

export function createMailboxStatus(
  session: OauthMailboxSessionSupported,
  overrides?: Partial<OauthMailboxStatus>,
): OauthMailboxStatus {
  return {
    sessionId: session.sessionId,
    emailAddress: session.emailAddress,
    expiresAt: session.expiresAt,
    latestCode: null,
    invite: null,
    invited: false,
    ...overrides,
  }
}

export { AccountPoolStoryRouter }
