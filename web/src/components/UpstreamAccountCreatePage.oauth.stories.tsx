import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, within } from 'storybook/test'
import type { OauthMailboxSession } from '../lib/api'
import { createPendingSession } from './UpstreamAccountsPage.story-data'
import {
  AccountPoolStoryRouter,
  UpstreamAccountCreatePage,
  createMailboxSession,
  createMailboxStatus,
  upstreamAccountCreateMetaBase,
} from './UpstreamAccountCreatePage.story-common'

const meta = {
  ...upstreamAccountCreateMetaBase,
  title: 'Account Pool/Pages/Upstream Account Create/OAuth',
} satisfies Meta<typeof UpstreamAccountCreatePage>

export default meta

type Story = StoryObj<typeof meta>

export const Ready: Story = {
  render: () => <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts/new" />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.type(canvas.getByLabelText(/display name/i), 'Codex Pro - Manual')
    await userEvent.click(canvas.getByRole('button', { name: /generate oauth url/i }))
    await expect(canvas.getByRole('button', { name: /copy oauth url/i })).toBeInTheDocument()
    await expect(canvas.getByLabelText(/callback url/i)).toBeInTheDocument()
  },
}

export const BlockedByUnselectableGroupProxy: Story = {
  render: () => (
    <AccountPoolStoryRouter
      initialEntry={{
        pathname: '/account-pool/upstream-accounts/new',
        state: {
          draft: {
            oauth: {
              displayName: 'Blocked OAuth Proxy',
              groupName: 'staging',
            },
          },
        },
      }}
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(
      canvas.getByText(/group "staging" does not have any selectable bound proxy nodes\./i),
    ).toBeInTheDocument()
    await expect(canvas.getByRole('button', { name: /generate oauth url/i })).toBeDisabled()
  },
}

export const DraftGroupBindingsUnlockGenerate: Story = {
  render: () => (
    <AccountPoolStoryRouter
      initialEntry={{
        pathname: '/account-pool/upstream-accounts/new',
        state: {
          draft: {
            oauth: {
              displayName: 'Launch Team Draft',
              groupName: 'launch-team',
            },
          },
        },
      }}
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const documentScope = within(canvasElement.ownerDocument.body)
    const generateButton = canvas.getByRole('button', { name: /generate oauth url/i })

    await expect(
      canvas.getByText(/group "launch-team" does not have any bound proxy nodes\./i),
    ).toBeInTheDocument()
    await expect(generateButton).toBeDisabled()

    await userEvent.click(
      canvas.getByRole('button', { name: /edit group settings|edit group note/i }),
    )
    const dialog = documentScope.getByRole('dialog', { name: /group settings|group note/i })
    await userEvent.click(within(dialog).getByRole('button', { name: /^direct/i }))
    await userEvent.click(within(dialog).getByRole('button', { name: /^save$/i }))

    await expect(
      canvas.queryByText(/group "launch-team" does not have any bound proxy nodes\./i),
    ).not.toBeInTheDocument()
    await expect(generateButton).toBeEnabled()

    await userEvent.click(generateButton)
    await expect(canvas.getByRole('button', { name: /copy oauth url/i })).toBeInTheDocument()
  },
}

export const PendingMetadataSync: Story = {
  render: () => (
    <AccountPoolStoryRouter
      initialEntry={{
        pathname: '/account-pool/upstream-accounts/new',
        state: {
          draft: {
            oauth: {
              displayName: 'Pending OAuth Sync',
              groupName: 'staging',
              note: 'Before metadata edit',
            },
          },
        },
      }}
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const displayName = canvas.getByLabelText(/display name/i)

    await userEvent.click(canvas.getByRole('button', { name: /generate oauth url/i }))
    await expect(canvas.getByRole('button', { name: /copy oauth url/i })).toBeEnabled()
    await userEvent.clear(displayName)
    await userEvent.type(displayName, 'Pending OAuth Sync Updated')

    await expect(canvas.getByRole('button', { name: /copy oauth url/i })).toBeEnabled()
    await expect(canvas.queryByText(/generate a fresh oauth url/i)).not.toBeInTheDocument()
  },
}

export const MailboxGenerated: Story = {
  render: () => <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts/new" />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(canvas.getByRole('button', { name: /^generate$/i }))

    const displayName = canvas.getByLabelText(/display name/i) as HTMLInputElement
    const copyMailboxButton = canvas.getByRole('button', { name: /copy mailbox/i })

    await expect(displayName.value).toMatch(/storybook-oauth-\d+@mail-tw\.707079\.xyz/i)
    await expect(copyMailboxButton).toBeInTheDocument()
    await expect(canvas.getByText(/storybook-oauth-\d+@mail-tw\.707079\.xyz/i)).toBeInTheDocument()
    await userEvent.hover(copyMailboxButton)
    await expect(within(document.body).getByText(/click to copy/i)).toBeInTheDocument()
  },
}

export const MailboxAttachFlow: Story = {
  render: () => (
    <AccountPoolStoryRouter
      initialEntry={{
        pathname: '/account-pool/upstream-accounts/new',
        state: {
          draft: {
            oauth: {
              displayName: 'Attach Mailbox Flow',
              groupName: 'production',
              mailboxInput: 'flow-oauth@mail-tw.707079.xyz',
            },
          },
        },
      }}
    />
  ),
}

export const MailboxAttachPending: Story = {
  render: () => (
    <AccountPoolStoryRouter
      initialEntry={{
        pathname: '/account-pool/upstream-accounts/new',
        state: {
          draft: {
            oauth: {
              displayName: 'Pending Attached Mailbox',
              groupName: 'production',
              mailboxInput: 'pending-oauth@mail-tw.707079.xyz',
              mailboxBusyAction: 'attach',
            },
          },
        },
      }}
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const useAddressButton = canvas.getByRole('button', { name: /use address/i })
    const generateButton = canvas.getByRole('button', { name: /^generate$/i })

    await expect(useAddressButton).toBeDisabled()
    await expect(generateButton).toBeDisabled()
    await expect(useAddressButton.querySelector('.animate-spin')).not.toBeNull()
    await expect(generateButton.querySelector('.animate-spin')).toBeNull()
  },
}

export const MailboxGenerateFlow: Story = {
  render: () => (
    <AccountPoolStoryRouter
      initialEntry={{
        pathname: '/account-pool/upstream-accounts/new',
        state: {
          draft: {
            oauth: {
              displayName: 'Generate Mailbox Flow',
              groupName: 'production',
            },
          },
        },
      }}
    />
  ),
}

export const MailboxGeneratePending: Story = {
  render: () => (
    <AccountPoolStoryRouter
      initialEntry={{
        pathname: '/account-pool/upstream-accounts/new',
        state: {
          draft: {
            oauth: {
              displayName: 'Pending Generated Mailbox',
              groupName: 'production',
              mailboxInput: 'pending-generated@mail-tw.707079.xyz',
              mailboxBusyAction: 'generate',
            },
          },
        },
      }}
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const useAddressButton = canvas.getByRole('button', { name: /use address/i })
    const generateButton = canvas.getByRole('button', { name: /^generate$/i })

    await expect(useAddressButton).toBeDisabled()
    await expect(generateButton).toBeDisabled()
    await expect(generateButton.querySelector('.animate-spin')).not.toBeNull()
    await expect(useAddressButton.querySelector('.animate-spin')).toBeNull()
  },
}

export const ManualMailboxUnsupported: Story = {
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

export const ReauthManualMailboxAttached: Story = {
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

export const ManualMailboxAutoCreated: Story = {
  render: () => <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts/new" />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const mailboxInput = canvas.getByPlaceholderText(/enter a supported mailbox address/i)
    await userEvent.clear(mailboxInput)
    await userEvent.type(mailboxInput, 'finance.lab.d5r@mail-tw.707079.xyz')
    await userEvent.click(canvas.getByRole('button', { name: /use address/i }))
    await expect(canvas.getByText(/finance\.lab\.d5r@mail-tw\.707079\.xyz/i)).toBeInTheDocument()
    await expect(canvas.getByText(/generated mailbox/i)).toBeInTheDocument()
    await userEvent.click(canvas.getByRole('button', { name: /generate oauth url/i }))
    await expect(canvas.getByRole('button', { name: /copy oauth url/i })).toBeEnabled()
  },
}

export const MailboxReady: Story = {
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

export const MailboxExpired: Story = {
  render: () => {
    const mailboxSession: OauthMailboxSession = {
      supported: true,
      sessionId: 'story-mailbox-oauth-expired',
      emailAddress: 'expired-oauth@mail-tw.707079.xyz',
      expiresAt: '2026-03-11T10:00:00.000Z',
      source: 'generated',
    }
    return (
      <AccountPoolStoryRouter
        initialEntry={{
          pathname: '/account-pool/upstream-accounts/new',
          state: {
            draft: {
              oauth: {
                displayName: 'Expired OAuth Mailbox',
                groupName: 'production',
                mailboxSession,
                mailboxInput: mailboxSession.emailAddress,
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

export const MailboxRefreshFailed: Story = {
  render: () => {
    const mailboxSession = createMailboxSession('story-mailbox-oauth-failed', 'failed-oauth@mail-tw.707079.xyz')
    return (
      <AccountPoolStoryRouter
        initialEntry={{
          pathname: '/account-pool/upstream-accounts/new',
          state: {
            draft: {
              oauth: {
                displayName: 'Failed OAuth Mailbox',
                groupName: 'production',
                mailboxSession,
                mailboxInput: mailboxSession.emailAddress,
                mailboxStatus: createMailboxStatus(mailboxSession, {
                  error: 'Mailbox refresh failed. We could not confirm the latest code or invite state.',
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
    await expect(canvas.getByText(/mailbox refresh failed/i)).toBeInTheDocument()
  },
}

export const MailboxDetachedName: Story = {
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

export const NameConflict: Story = {
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

export const DuplicateWarning: Story = {
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
