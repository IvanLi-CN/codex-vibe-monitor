import type { Meta, StoryObj } from '@storybook/react-vite'
import { userEvent, within, expect } from 'storybook/test'
import { SystemNotificationProvider } from './ui/system-notifications'
import { I18nProvider } from '../i18n'
import UpstreamAccountCreatePage from '../pages/account-pool/UpstreamAccountCreate'
import type { OauthMailboxSession, OauthMailboxSessionSupported, OauthMailboxStatus } from '../lib/api'
import { Alert } from './ui/alert'
import { Badge } from './ui/badge'
import { Button } from './ui/button'
import { Spinner } from './ui/spinner'
import { AppIcon } from './AppIcon'
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

function createMailboxSession(sessionId: string, emailAddress: string): OauthMailboxSessionSupported {
  return {
    supported: true,
    sessionId,
    emailAddress,
    expiresAt: '2026-03-20T12:50:00.000Z',
    source: 'generated',
  }
}

function createMailboxStatus(
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

const oauthMailboxGalleryStates = [
  {
    title: '生成后等待邮件',
    description: '已生成临时邮箱，但验证码和邀请都还没到。',
    codeSubtitle: '暂时还没有识别到验证码。',
    codeValue: '—',
    inviteSubtitle: '暂时还没有识别到邀请通知。',
    inviteValue: '—',
    invitedLabel: '未受邀',
    invitedVariant: 'secondary' as const,
  },
  {
    title: '查收中',
    description: '邮箱状态轮询进行中，标题旁显示查收中标记。',
    codeBadge: 'checking' as const,
    codeSubtitle: '暂时还没有识别到验证码。',
    codeValue: '—',
    inviteSubtitle: '暂时还没有识别到邀请通知。',
    inviteValue: '—',
    invitedLabel: '未受邀',
    invitedVariant: 'secondary' as const,
  },
  {
    title: '验证码已就绪',
    description: '验证码、邀请摘要和收到时间都齐全的正常态。',
    codeSubtitle: '收到于 2026/03/11 20:36:00',
    codeValue: '824931',
    inviteSubtitle: '收到于 2026/03/11 20:37:00',
    inviteValue: 'https://chatgpt.com/invite/story-ready',
    invitedLabel: '已受邀',
    invitedVariant: 'success' as const,
  },
  {
    title: '查收失败',
    description: '轮询失败时显示查收失败标记，并保留错误提示。',
    issueVariant: 'error' as const,
    issueText: '邮箱状态刷新失败，暂时无法确认最新验证码或邀请状态。',
    codeBadge: 'failed' as const,
    codeSubtitle: '暂时还没有识别到验证码。',
    codeValue: '—',
    inviteSubtitle: '暂时还没有识别到邀请通知。',
    inviteValue: '—',
    invitedLabel: '未受邀',
    invitedVariant: 'secondary' as const,
  },
  {
    title: '邮箱已过期',
    description: '临时邮箱过期后停止查收，并给出过期提示。',
    issueVariant: 'warning' as const,
    issueText: '这个临时邮箱已经过期了。请重新生成一个新邮箱再等新邮件。',
    codeSubtitle: '暂时还没有识别到验证码。',
    codeValue: '—',
    inviteSubtitle: '暂时还没有识别到邀请通知。',
    inviteValue: '—',
    invitedLabel: '未受邀',
    invitedVariant: 'secondary' as const,
  },
] as const

function OauthMailboxStateCard({
  title,
  description,
  issueText,
  issueVariant,
  codeBadge,
  codeSubtitle,
  codeValue,
  inviteSubtitle,
  inviteValue,
  invitedLabel,
  invitedVariant,
}: {
  title: string
  description: string
  issueText?: string
  issueVariant?: 'warning' | 'error'
  codeBadge?: 'checking' | 'failed'
  codeSubtitle: string
  codeValue: string
  inviteSubtitle: string
  inviteValue: string
  invitedLabel: string
  invitedVariant: 'secondary' | 'success'
}) {
  return (
    <section className="overflow-hidden rounded-[28px] border border-base-300/80 bg-base-100 shadow-[0_24px_64px_-36px_rgba(15,23,42,0.35)]">
      <div className="border-b border-base-300/70 bg-gradient-to-r from-base-200/80 via-base-100 to-base-100 px-5 py-4">
        <p className="text-sm font-semibold text-base-content">{title}</p>
        <p className="mt-1 text-sm text-base-content/65">{description}</p>
      </div>
      <div className="space-y-4 bg-base-100 p-5">
        {issueText && issueVariant ? <Alert variant={issueVariant}>{issueText}</Alert> : null}
        <div className="grid gap-4 lg:grid-cols-2">
          <div className="rounded-2xl border border-base-300/70 bg-base-200/40 p-4">
            <div className="flex items-center justify-between gap-3">
              <div>
                <p className="flex items-center gap-2 text-sm font-semibold text-base-content">
                  验证码
                  {codeBadge === 'checking' ? (
                    <Badge
                      variant="secondary"
                      className="h-5 gap-1 rounded-full px-1.5 py-0 text-[10px] font-medium leading-none"
                    >
                      <Spinner size="sm" className="h-2.5 w-2.5" />
                      查收中
                    </Badge>
                  ) : null}
                  {codeBadge === 'failed' ? (
                    <Badge
                      variant="error"
                      className="h-5 rounded-full px-1.5 py-0 text-[10px] font-medium leading-none"
                    >
                      查收失败
                    </Badge>
                  ) : null}
                </p>
                <p className="mt-1 text-xs text-base-content/65">{codeSubtitle}</p>
              </div>
              <Button type="button" size="sm" variant="default" disabled={codeValue === '—'}>
                <AppIcon name="content-copy" className="mr-1.5 h-4 w-4" aria-hidden />
                复制验证码
              </Button>
            </div>
            <p className="mt-4 font-mono text-2xl font-semibold tracking-[0.24em] text-base-content">{codeValue}</p>
          </div>
          <div className="rounded-2xl border border-base-300/70 bg-base-200/40 p-4">
            <div className="flex items-center justify-between gap-3">
              <div>
                <p className="text-sm font-semibold text-base-content">邀请摘要</p>
                <p className="mt-1 text-xs text-base-content/65">{inviteSubtitle}</p>
              </div>
              <Button type="button" variant="secondary" size="sm" disabled={inviteValue === '—'}>
                <AppIcon name="content-copy" className="mr-1.5 h-4 w-4" aria-hidden />
                复制邀请
              </Button>
            </div>
            <div className="mt-4 flex items-center gap-3">
              <Badge variant={invitedVariant} className="rounded-full px-3 py-1 text-sm">
                {invitedLabel}
              </Badge>
              <span className="truncate text-sm text-base-content/70">{inviteValue}</span>
            </div>
          </div>
        </div>
      </div>
    </section>
  )
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

export const OauthManualMailboxUnsupported: Story = {
  name: 'OAuth Manual Mailbox Unsupported',
  render: () => <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts/new" />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.type(canvas.getByPlaceholderText(/enter a supported mailbox address/i), 'manual@example.com')
    await userEvent.click(canvas.getByRole('button', { name: /use address/i }))
    await expect(
      canvas.getByText(/mailbox is not readable through the current moemail integration/i),
    ).toBeInTheDocument()
    await expect(canvas.getByRole('button', { name: /generate oauth url/i })).toBeEnabled()
  },
}

export const OauthReauthManualMailboxAttached: Story = {
  name: 'OAuth Reauth Manual Mailbox Attached',
  render: () => (
    <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts/new?mode=oauth&accountId=101" />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const mailboxInput = canvas.getByPlaceholderText(/enter a supported mailbox address/i)
    await userEvent.clear(mailboxInput)
    await userEvent.type(mailboxInput, 'manual-existing@mail-tw.707079.xyz')
    await userEvent.click(canvas.getByRole('button', { name: /use address/i }))
    await expect(canvas.getByText(/manual-existing@mail-tw\.707079\.xyz/i)).toBeInTheDocument()
    await expect(canvas.getByText(/attached/i)).toBeInTheDocument()
    await userEvent.click(canvas.getByRole('button', { name: /generate oauth url/i }))
    await expect(canvas.getByRole('button', { name: /copy oauth url/i })).toBeEnabled()
  },
}

export const OauthMailboxStateGallery: Story = {
  name: 'OAuth Mailbox State Gallery',
  parameters: {
    docs: {
      description: {
        story:
          'Curated single-account mailbox docs view that concentrates the mailbox lifecycle, status badges, and failure handling into one Storybook surface.',
      },
    },
  },
  render: () => (
    <div className="min-h-screen bg-[radial-gradient(circle_at_top,_rgba(59,130,246,0.16),_transparent_52%),linear-gradient(180deg,rgba(248,250,252,1)_0%,rgba(241,245,249,1)_100%)] px-6 py-8 text-base-content">
      <div className="mx-auto max-w-[1720px] space-y-6">
        <div className="space-y-2">
          <p className="text-xs font-semibold uppercase tracking-[0.28em] text-primary/80">Single OAuth Mailbox</p>
          <h2 className="text-3xl font-semibold tracking-tight text-base-content">单账号邮箱状态总览</h2>
          <p className="max-w-3xl text-sm leading-6 text-base-content/70">
            把单账号 OAuth 邮箱的等待、查收中、验证码就绪、查收失败和过期集中到一页里，专门用来评审邮箱区块的多状态视觉与文案。
          </p>
        </div>
        <div className="grid gap-6 xl:grid-cols-2">
          {oauthMailboxGalleryStates.map((item) => (
            <OauthMailboxStateCard key={item.title} {...item} />
          ))}
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

    await expect(displayName.value).toMatch(/storybook-oauth-\d+@mail-tw\.707079\.xyz/i)
    await expect(canvas.getAllByRole('button', { name: /copy mailbox/i })[0]).toBeInTheDocument()
    await expect(canvas.getByText(/storybook-oauth-\d+@mail-tw\.707079\.xyz/i)).toBeInTheDocument()
  },
}

export const BatchOauthActionTooltips: Story = {
  name: 'Batch OAuth Action Tooltips',
  render: () => <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts/new?mode=batchOauth" />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)

    const generateButton = canvas.getByRole('button', { name: /generate oauth url/i })
    await userEvent.hover(generateButton)
    await expect(within(document.body).getByText(/generate a fresh oauth url for this row/i)).toBeInTheDocument()

    const copyCodeButton = canvas.getByRole('button', { name: /copy verification code/i })
    const tooltipTrigger = copyCodeButton.parentElement
    if (!(tooltipTrigger instanceof HTMLElement)) {
      throw new Error('missing tooltip trigger for copy verification code button')
    }

    await userEvent.hover(tooltipTrigger)
    await expect(within(document.body).getByText(/no verification code yet/i)).toBeInTheDocument()
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
    await expect(canvas.getByRole('button', { name: /copy verification code/i })).toHaveTextContent('556677')
  },
}

export const BatchOauthInvitedBadgeTooltip: Story = {
  name: 'Batch OAuth Invited Badge Tooltip',
  render: () => {
    const mailboxSession = createMailboxSession('story-mailbox-batch-invited', 'batch-invited@mail-tw.707079.xyz')
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
                    displayName: 'Invited Batch Mailbox',
                    groupName: 'production',
                    mailboxSession,
                    mailboxInput: mailboxSession.emailAddress,
                    mailboxStatus: createMailboxStatus(mailboxSession, {
                      invited: true,
                    }),
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
    const invitedBadge = canvas.getByRole('button', { name: /invite received/i })
    await userEvent.hover(invitedBadge)
    await expect(within(document.body).getByText(/this mailbox already received a workspace invite email/i)).toBeInTheDocument()
  },
}

export const BatchOauthMailboxExpired: Story = {
  name: 'Batch OAuth Mailbox Expired',
  render: () => {
    const mailboxSession: OauthMailboxSession = {
      supported: true,
      sessionId: 'story-mailbox-batch-expired',
      emailAddress: 'expired-batch@mail-tw.707079.xyz',
      expiresAt: '2026-03-11T10:00:00.000Z',
      source: 'generated',
    }
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
                    displayName: 'Expired Batch Mailbox',
                    groupName: 'production',
                    mailboxSession,
                    mailboxInput: mailboxSession.emailAddress,
                    mailboxError: 'This temp mailbox has expired. Generate a fresh mailbox before waiting for new mail.',
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
    await expect(canvas.getByText(/temp mailbox has expired/i)).toBeInTheDocument()
  },
}

export const BatchOauthMailboxRefreshFailed: Story = {
  name: 'Batch OAuth Mailbox Refresh Failed',
  render: () => {
    const mailboxSession = createMailboxSession('story-mailbox-batch-failed', 'failed-batch@mail-tw.707079.xyz')
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
                    displayName: 'Failed Batch Mailbox',
                    groupName: 'production',
                    mailboxSession,
                    mailboxInput: mailboxSession.emailAddress,
                    mailboxStatus: createMailboxStatus(mailboxSession, {
                      error: 'Mailbox refresh failed. We could not confirm the latest code or invite state.',
                    }),
                    mailboxError: 'Mailbox refresh failed. We could not confirm the latest code or invite state.',
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
    await expect(canvas.getByText(/mailbox refresh failed/i)).toBeInTheDocument()
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
    await expect(canvas.getByRole('button', { name: /copy verification code/i })).toHaveTextContent('334455')
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
