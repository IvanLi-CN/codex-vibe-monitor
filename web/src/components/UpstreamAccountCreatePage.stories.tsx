import type { Meta, StoryObj } from '@storybook/react-vite'
import { userEvent, within, expect } from 'storybook/test'
import { SystemNotificationProvider } from './ui/system-notifications'
import { I18nProvider } from '../i18n'
import UpstreamAccountCreatePage from '../pages/account-pool/UpstreamAccountCreate'
import type { OauthMailboxSession, OauthMailboxStatus } from '../lib/api'
import { OauthMailboxChip } from './account-pool/OauthMailboxChip'
import {
  AccountPoolStoryRouter,
  StorybookUpstreamAccountsMock,
} from './UpstreamAccountsPage.story-helpers'
import { createPendingSession } from './UpstreamAccountsPage.story-data'

function createCompletedSession(loginId: string, accountId: number) {
  return {
    loginId,
    status: 'completed' as const,
    authUrl: null,
    redirectUri: null,
    expiresAt: '2026-03-11T13:30:00.000Z',
    accountId,
    error: null,
  }
}

function createMailboxSession(sessionId: string, emailAddress: string): OauthMailboxSession {
  return {
    sessionId,
    emailAddress,
    expiresAt: '2026-03-11T12:50:00.000Z',
  }
}

function createMailboxStatus(
  session: OauthMailboxSession,
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

const meta = {
  title: 'Account Pool/Pages/Upstream Account Create',
  component: UpstreamAccountCreatePage,
  tags: ['autodocs'],
  parameters: {
    layout: 'fullscreen',
  },
  decorators: [
    (Story) => (
      <I18nProvider>
        <SystemNotificationProvider>
          <StorybookUpstreamAccountsMock>
            <Story />
          </StorybookUpstreamAccountsMock>
        </SystemNotificationProvider>
      </I18nProvider>
    ),
  ],
} satisfies Meta<typeof UpstreamAccountCreatePage>

export default meta

type Story = StoryObj<typeof meta>

export const Default: Story = {
  render: () => <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts/new" />,
}

export const OauthReady: Story = {
  render: () => <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts/new" />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.type(canvas.getByLabelText(/display name/i), 'Codex Pro - Manual')
    await userEvent.click(canvas.getByRole('button', { name: /generate oauth url/i }))
    await expect(canvas.getByRole('button', { name: /copy oauth url/i })).toBeInTheDocument()
    await expect(canvas.getByLabelText(/callback url/i)).toBeInTheDocument()
  },
}

export const OauthMailboxGenerated: Story = {
  name: 'OAuth Mailbox Generated',
  render: () => <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts/new" />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(canvas.getByRole('button', { name: /^generate$/i }))

    const displayName = canvas.getByLabelText(/display name/i) as HTMLInputElement
    const copyMailboxButton = canvas.getByRole('button', { name: /copy mailbox/i })

    await expect(displayName.value).toBe('')
    await expect(copyMailboxButton).toBeInTheDocument()
    await expect(canvas.getByText(/storybook-oauth-\d+@mail-tw\.707079\.xyz/i)).toBeInTheDocument()
    await userEvent.hover(copyMailboxButton)
    await expect(within(document.body).getByText(/click to copy/i)).toBeInTheDocument()
  },
}

export const OauthMailboxReady: Story = {
  name: 'OAuth Mailbox Ready',
  render: () => {
    const mailboxSession = createMailboxSession('story-mailbox-oauth-ready', 'oauth-ready@mail-tw.707079.xyz')
    return (
      <AccountPoolStoryRouter
        initialEntry={{
          pathname: '/account-pool/upstream-accounts/new',
          state: {
            draft: {
              oauth: {
                displayName: 'Codex Pro - Manual',
                groupName: 'production',
                mailboxSession,
                mailboxInput: mailboxSession.emailAddress,
                mailboxStatus: createMailboxStatus(mailboxSession, {
                  latestCode: {
                    value: '824931',
                    source: 'subject',
                    updatedAt: '2026-03-11T12:36:00.000Z',
                  },
                  invite: {
                    subject: 'Alice has invited you to join OpenAI Workspace',
                    copyValue: 'https://chatgpt.com/invite/story-ready',
                    copyLabel: 'Join workspace',
                    updatedAt: '2026-03-11T12:37:00.000Z',
                  },
                  invited: true,
                }),
              },
            },
          },
        }}
      />
    )
  },
}

export const OauthMailboxHover: Story = {
  name: 'OAuth Mailbox Hover',
  render: () => (
    <div className="min-h-screen bg-base-200 px-10 py-12">
      <div className="max-w-xl rounded-2xl border border-base-300/80 bg-base-100 p-6 shadow-sm">
        <div className="flex items-center gap-3">
          <span className="field-label shrink-0">Display Name</span>
          <OauthMailboxChip
            className="max-w-[24rem]"
            emailAddress="hover-preview@mail-tw.707079.xyz"
            emptyLabel="No mailbox yet"
            copyAriaLabel="Copy mailbox"
            copyHintLabel="Click to copy"
            copiedLabel="Copied"
            manualCopyLabel="Auto copy failed. Please copy the mailbox below manually."
            manualBadgeLabel="Manual"
            onCopy={() => undefined}
          />
        </div>
      </div>
    </div>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const copyMailboxButton = canvas.getByRole('button', { name: /copy mailbox/i })

    await userEvent.hover(copyMailboxButton)
    const tooltip = within(document.body)
    await expect(tooltip.getByText(/click to copy/i)).toBeInTheDocument()
    await expect(tooltip.getByText(/hover-preview@mail-tw\.707079\.xyz/i)).toBeInTheDocument()
  },
}

export const OauthMailboxLongPress: Story = {
  name: 'OAuth Mailbox Long Press',
  render: () => (
    <div className="min-h-screen bg-base-200 px-10 py-12">
      <div className="max-w-xl rounded-2xl border border-base-300/80 bg-base-100 p-6 shadow-sm">
        <div className="flex items-center gap-3">
          <span className="field-label shrink-0">Display Name</span>
          <OauthMailboxChip
            className="max-w-[24rem]"
            emailAddress="press-preview@mail-tw.707079.xyz"
            emptyLabel="No mailbox yet"
            copyAriaLabel="Copy mailbox"
            copyHintLabel="Click to copy"
            copiedLabel="Copied"
            manualCopyLabel="Auto copy failed. Please copy the mailbox below manually."
            manualBadgeLabel="Manual"
            onCopy={() => undefined}
          />
        </div>
      </div>
    </div>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const copyMailboxButton = canvas.getByRole('button', { name: /copy mailbox/i })

    copyMailboxButton.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, pointerType: 'touch', button: 0 }))
    await new Promise((resolve) => window.setTimeout(resolve, 420))

    const tooltip = within(document.body)
    await expect(tooltip.getByText(/click to copy/i)).toBeInTheDocument()
    await expect(tooltip.getByText(/press-preview@mail-tw\.707079\.xyz/i)).toBeInTheDocument()

    copyMailboxButton.dispatchEvent(new PointerEvent('pointerup', { bubbles: true, pointerType: 'touch', button: 0 }))
  },
}

export const OauthMailboxCopied: Story = {
  name: 'OAuth Mailbox Copied',
  render: () => (
    <div className="min-h-screen bg-base-200 px-10 py-12">
      <div className="max-w-xl rounded-2xl border border-base-300/80 bg-base-100 p-6 shadow-sm">
        <div className="flex items-center gap-3">
          <span className="field-label shrink-0">Display Name</span>
          <OauthMailboxChip
            className="max-w-[24rem]"
            emailAddress="copied-preview@mail-tw.707079.xyz"
            emptyLabel="No mailbox yet"
            copyAriaLabel="Copy mailbox"
            copyHintLabel="Click to copy"
            copiedLabel="Copied"
            manualCopyLabel="Auto copy failed. Please copy the mailbox below manually."
            manualBadgeLabel="Manual"
            tone="copied"
            onCopy={() => undefined}
          />
        </div>
      </div>
    </div>
  ),
}

export const OauthMailboxManualCopy: Story = {
  name: 'OAuth Mailbox Manual Copy',
  render: () => (
    <div className="min-h-screen bg-base-200 px-10 py-12">
      <div className="max-w-xl rounded-2xl border border-base-300/80 bg-base-100 p-6 shadow-sm">
        <div className="flex items-center gap-3">
          <span className="field-label shrink-0">Display Name</span>
          <OauthMailboxChip
            className="max-w-[24rem]"
            emailAddress="manual-copy@mail-tw.707079.xyz"
            emptyLabel="No mailbox yet"
            copyAriaLabel="Copy mailbox"
            copyHintLabel="Click to copy"
            copiedLabel="Copied"
            manualCopyLabel="Auto copy failed. Please copy the mailbox below manually."
            manualBadgeLabel="Manual"
            tone="manual"
            onCopy={() => undefined}
          />
        </div>
      </div>
    </div>
  ),
}

export const OauthMailboxDetachedName: Story = {
  name: 'OAuth Mailbox Detached Name',
  render: () => {
    const mailboxSession = createMailboxSession('story-mailbox-oauth-mismatch', 'oauth-lock@mail-tw.707079.xyz')
    return (
      <AccountPoolStoryRouter
        initialEntry={{
          pathname: '/account-pool/upstream-accounts/new',
          state: {
            draft: {
              oauth: {
                displayName: 'manual-alias@mail-tw.707079.xyz',
                groupName: 'production',
                mailboxSession,
                mailboxInput: mailboxSession.emailAddress,
                mailboxStatus: createMailboxStatus(mailboxSession, {
                  latestCode: {
                    value: '190284',
                    source: 'body',
                    updatedAt: '2026-03-11T12:36:00.000Z',
                  },
                  invite: {
                    subject: 'Alice has invited you to join OpenAI Workspace',
                    copyValue: 'https://chatgpt.com/invite/story-locked',
                    copyLabel: 'Join workspace',
                    updatedAt: '2026-03-11T12:37:00.000Z',
                  },
                  invited: true,
                }),
              },
            },
          },
        }}
      />
    )
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByRole('button', { name: /copy verification code/i })).toBeEnabled()
    await expect(canvas.getByRole('button', { name: /copy invite/i })).toBeEnabled()
  },
}

export const BatchOauthReady: Story = {
  render: () => <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts/new?mode=batchOauth" />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(canvas.getByRole('button', { name: /generate oauth url/i }))
    await expect(canvas.getByDisplayValue(/https:\/\/auth\.openai\.com\/authorize/i)).toBeInTheDocument()
    await expect(canvas.getByRole('button', { name: /complete oauth login/i })).toBeInTheDocument()
  },
}

export const BatchOauthMailboxGenerated: Story = {
  name: 'Batch OAuth Mailbox Generated',
  render: () => <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts/new?mode=batchOauth" />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(canvas.getAllByRole('button', { name: /^generate$/i })[0])

    const displayName = canvas.getAllByLabelText(/display name/i)[0] as HTMLInputElement

    await expect(displayName.value).toBe('')
    await expect(canvas.getAllByRole('button', { name: /copy mailbox/i })[0]).toBeInTheDocument()
    await expect(canvas.getByText(/storybook-oauth-\d+@mail-tw\.707079\.xyz/i)).toBeInTheDocument()
  },
}

export const BatchOauthMailboxReady: Story = {
  name: 'Batch OAuth Mailbox Ready',
  render: () => {
    const mailboxSession = createMailboxSession('story-mailbox-batch-ready', 'batch-row@mail-tw.707079.xyz')
    return (
      <AccountPoolStoryRouter
        initialEntry={{
          pathname: '/account-pool/upstream-accounts/new',
          search: '?mode=batchOauth',
          state: {
            draft: {
              batchOauth: {
                defaultGroupName: 'production',
                rows: [
                  {
                    id: 'row-1',
                    displayName: 'Batch Row One',
                    groupName: 'production',
                    mailboxSession,
                    mailboxInput: mailboxSession.emailAddress,
                    mailboxCodeTone: 'idle',
                    mailboxStatus: createMailboxStatus(mailboxSession, {
                      latestCode: {
                        value: '556677',
                        source: 'subject',
                        updatedAt: '2026-03-11T12:36:00.000Z',
                      },
                      invite: {
                        subject: 'Alice has invited you to join OpenAI Workspace',
                        copyValue: 'https://chatgpt.com/invite/batch-ready',
                        copyLabel: 'Join workspace',
                        updatedAt: '2026-03-11T12:37:00.000Z',
                      },
                      invited: true,
                    }),
                  },
                  {
                    id: 'row-2',
                    displayName: 'Codex Pro - Spare',
                    groupName: 'production',
                  },
                ],
              },
            },
          },
        }}
      />
    )
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByRole('button', { name: /copy verification code/i })).toBeEnabled()
  },
}

export const BatchOauthMailboxDetachedName: Story = {
  name: 'Batch OAuth Mailbox Detached Name',
  render: () => {
    const mailboxSession = createMailboxSession('story-mailbox-batch-mismatch', 'batch-locked@mail-tw.707079.xyz')
    return (
      <AccountPoolStoryRouter
        initialEntry={{
          pathname: '/account-pool/upstream-accounts/new',
          search: '?mode=batchOauth',
          state: {
            draft: {
              batchOauth: {
                defaultGroupName: 'production',
                rows: [
                  {
                    id: 'row-1',
                    displayName: 'manual-alias@mail-tw.707079.xyz',
                    groupName: 'production',
                    mailboxSession,
                    mailboxInput: mailboxSession.emailAddress,
                    mailboxStatus: createMailboxStatus(mailboxSession, {
                      latestCode: {
                        value: '334455',
                        source: 'body',
                        updatedAt: '2026-03-11T12:36:00.000Z',
                      },
                      invited: true,
                    }),
                  },
                  {
                    id: 'row-2',
                    displayName: 'Codex Pro - Spare',
                    groupName: 'production',
                  },
                ],
              },
            },
          },
        }}
      />
    )
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByRole('button', { name: /copy verification code/i })).toBeEnabled()
  },
}

export const OauthNameConflict: Story = {
  name: 'OAuth Name Conflict',
  render: () => (
    <AccountPoolStoryRouter
      initialEntry={{
        pathname: '/account-pool/upstream-accounts/new',
        state: {
          draft: {
            oauth: {
              displayName: ' codex pro - tokyo ',
              groupName: 'production',
              note: 'Conflicts with an existing OAuth account name.',
              callbackUrl: 'http://localhost:43210/oauth/callback?code=oauth-duplicate&state=storybook',
              session: createPendingSession('story-oauth-duplicate'),
              sessionHint: 'Pending OAuth session ready for duplicate-name review.',
            },
          },
        },
      }}
    />
  ),
}

export const OauthDuplicateWarning: Story = {
  name: 'OAuth Duplicate Warning',
  render: () => (
    <AccountPoolStoryRouter
      initialEntry={{
        pathname: '/account-pool/upstream-accounts/new',
        state: {
          draft: {
            oauth: {
              displayName: 'Codex Pro - Tokyo',
              groupName: 'production',
              note: 'Freshly linked account that shares the same upstream identity.',
              callbackUrl: 'http://localhost:43210/oauth/callback?code=oauth-duplicate&state=storybook',
              session: {
                loginId: 'story-oauth-duplicate-done',
                status: 'completed',
                authUrl: null,
                redirectUri: null,
                expiresAt: '2026-03-11T13:30:00.000Z',
                accountId: 101,
                error: null,
              },
              sessionHint: 'OAuth callback completed',
              duplicateWarning: {
                accountId: 101,
                displayName: 'Codex Pro - Tokyo',
                peerAccountIds: [103],
                reasons: ['sharedChatgptAccountId', 'sharedChatgptUserId'],
              },
            },
          },
        },
      }}
    />
  ),
}

export const BatchOauthNameConflict: Story = {
  name: 'Batch OAuth Name Conflict',
  render: () => (
    <AccountPoolStoryRouter
      initialEntry={{
        pathname: '/account-pool/upstream-accounts/new',
        search: '?mode=batchOauth',
        state: {
          draft: {
            batchOauth: {
              defaultGroupName: 'production',
              rows: [
                {
                  id: 'row-1',
                  displayName: ' Codex Pro - Tokyo ',
                  groupName: 'production',
                  note: 'Conflicts with an existing account in the pool.',
                  callbackUrl: 'http://localhost:43210/oauth/callback?code=batch-duplicate&state=storybook',
                  session: createPendingSession('story-batch-duplicate'),
                  sessionHint: 'Pending OAuth session ready for duplicate-name review.',
                },
                {
                  id: 'row-2',
                  displayName: 'Codex Pro - Osaka',
                  groupName: 'production',
                  note: 'Healthy comparison row.',
                },
              ],
            },
          },
        },
      }}
    />
  ),
}

export const BatchOauthDuplicateWarning: Story = {
  name: 'Batch OAuth Duplicate Warning',
  render: () => (
    <AccountPoolStoryRouter
      initialEntry={{
        pathname: '/account-pool/upstream-accounts/new',
        search: '?mode=batchOauth',
        state: {
          draft: {
            batchOauth: {
              defaultGroupName: 'production',
              rows: [
                {
                  id: 'row-1',
                  displayName: 'Codex Pro - Tokyo',
                  groupName: 'production',
                  note: 'Completed row with duplicate upstream identity.',
                  callbackUrl: 'http://localhost:43210/oauth/callback?code=batch-duplicate&state=storybook',
                  session: {
                    loginId: 'story-batch-duplicate-done',
                    status: 'completed',
                    authUrl: null,
                    redirectUri: null,
                    expiresAt: '2026-03-11T13:30:00.000Z',
                    accountId: 101,
                    error: null,
                  },
                  sessionHint: 'Codex Pro - Tokyo is ready. Continue with the remaining rows when you are done here.',
                  duplicateWarning: {
                    accountId: 101,
                    displayName: 'Codex Pro - Tokyo',
                    peerAccountIds: [103],
                    reasons: ['sharedChatgptAccountId', 'sharedChatgptUserId'],
                  },
                },
                {
                  id: 'row-2',
                  displayName: 'Codex Pro - Osaka',
                  groupName: 'production',
                  note: 'Healthy comparison row.',
                },
              ],
            },
          },
        },
      }}
    />
  ),
}

export const BatchOauthMixedStates: Story = {
  name: 'Batch OAuth Mixed States',
  render: () => (
    <AccountPoolStoryRouter
      initialEntry={{
        pathname: '/account-pool/upstream-accounts/new',
        search: '?mode=batchOauth',
        state: {
          draft: {
            batchOauth: {
              defaultGroupName: 'production',
              rows: [
                {
                  id: 'row-1',
                  displayName: 'Codex Pro - Tokyo',
                  groupName: 'production',
                  callbackUrl: 'http://localhost:43210/oauth/callback?code=batch-duplicate&state=storybook',
                  session: createCompletedSession('story-batch-duplicate-done', 101),
                  sessionHint: 'Codex Pro - Tokyo is ready. Continue with the remaining rows when you are done here.',
                  duplicateWarning: {
                    accountId: 101,
                    displayName: 'Codex Pro - Tokyo',
                    peerAccountIds: [103],
                    reasons: ['sharedChatgptAccountId', 'sharedChatgptUserId'],
                  },
                },
                {
                  id: 'row-2',
                  displayName: 'Codex Pro - Osaka',
                  groupName: 'production',
                  callbackUrl: 'http://localhost:43210/oauth/callback?code=batch-pending&state=storybook',
                  session: createPendingSession('story-batch-pending'),
                  sessionHint: 'Pending OAuth session ready for callback review.',
                },
                {
                  id: 'row-3',
                  displayName: 'Codex Pro - Nagoya',
                  groupName: 'production',
                  callbackUrl: 'http://localhost:43210/oauth/callback?code=batch-clean&state=storybook',
                  session: createCompletedSession('story-batch-clean-done', 104),
                  sessionHint: 'Codex Pro - Nagoya is ready. Continue with the remaining rows when you are done here.',
                },
                {
                  id: 'row-4',
                  displayName: 'Codex Pro - Fukuoka',
                  groupName: 'staging',
                },
              ],
            },
          },
        },
      }}
    />
  ),
}

export const ApiKeyNameConflict: Story = {
  name: 'API Key Name Conflict',
  render: () => (
    <AccountPoolStoryRouter
      initialEntry={{
        pathname: '/account-pool/upstream-accounts/new',
        search: '?mode=apiKey',
        state: {
          draft: {
            apiKey: {
              displayName: ' team key - staging ',
              groupName: 'staging',
              note: 'Conflicts with an existing API Key account name.',
              apiKeyValue: 'sk-storybookduplicate1234',
              primaryLimit: '120',
              secondaryLimit: '500',
              limitUnit: 'requests',
            },
          },
        },
      }}
    />
  ),
}
