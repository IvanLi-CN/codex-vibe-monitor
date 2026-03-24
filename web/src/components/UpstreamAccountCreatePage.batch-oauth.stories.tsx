import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, within } from 'storybook/test'
import type { OauthMailboxSession } from '../lib/api'
import { createPendingSession } from './UpstreamAccountsPage.story-data'
import {
  AccountPoolStoryRouter,
  UpstreamAccountCreatePage,
  createCompletedSession,
  createMailboxSession,
  createMailboxStatus,
  upstreamAccountCreateMetaBase,
} from './UpstreamAccountCreatePage.story-common'

const meta = {
  ...upstreamAccountCreateMetaBase,
  title: 'Account Pool/Pages/Upstream Account Create/Batch OAuth',
} satisfies Meta<typeof UpstreamAccountCreatePage>

export default meta

type Story = StoryObj<typeof meta>

export const Ready: Story = {
  render: () => <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts/new?mode=batchOauth" />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(canvas.getByRole('button', { name: /generate oauth url/i }))
    await expect(canvas.getByDisplayValue(/https:\/\/auth\.openai\.com\/authorize/i)).toBeInTheDocument()
    await expect(canvas.getByRole('button', { name: /complete oauth login/i })).toBeInTheDocument()
  },
}

export const MailboxGenerated: Story = {
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

export const MailboxPopoverEdit: Story = {
  render: () => {
    const mailboxSession = createMailboxSession('story-mailbox-batch-edit', 'batch-edit@mail-tw.707079.xyz')
    return (
      <AccountPoolStoryRouter
        initialEntry={{
          pathname: '/account-pool/upstream-accounts/new',
          search: '?mode=batchOauth',
          state: {
            draft: {
              batchOauth: {
                rows: [
                  {
                    id: 'row-1',
                    displayName: 'Batch Editable Mailbox',
                    groupName: 'production',
                    mailboxSession,
                    mailboxInput: mailboxSession.emailAddress,
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
    const mailboxChip = canvas.getByRole('button', { name: /copy mailbox/i })
    await userEvent.hover(mailboxChip)
    await expect(within(document.body).getByRole('button', { name: /edit mailbox/i })).toBeInTheDocument()
    await userEvent.click(within(document.body).getByRole('button', { name: /edit mailbox/i }))
    const editorInput = within(document.body).getByRole('textbox', { name: /mailbox address/i })
    const submitButton = within(document.body).getByRole('button', { name: /submit mailbox/i })
    await expect(editorInput).toBeInTheDocument()
    await expect(submitButton).toBeInTheDocument()
    await expect(within(document.body).getByRole('button', { name: /cancel mailbox edit/i })).toBeInTheDocument()
    await userEvent.clear(editorInput)
    await userEvent.type(editorInput, 'edited-batch@mail-tw.707079.xyz')
    await userEvent.click(submitButton)
    await expect(submitButton).toBeDisabled()
    await expect(canvas.getByText(/edited-batch@mail-tw\.707079\.xyz/i)).toBeInTheDocument()
  },
}

export const MailboxAttachFlow: Story = {
  render: () => (
    <AccountPoolStoryRouter
      initialEntry={{
        pathname: '/account-pool/upstream-accounts/new',
        search: '?mode=batchOauth',
        state: {
          draft: {
            batchOauth: {
              rows: [
                {
                  id: 'row-1',
                  displayName: 'Batch Attach Mailbox Flow',
                  groupName: 'production',
                  mailboxInput: 'flow-batch@mail-tw.707079.xyz',
                  mailboxEditorOpen: true,
                  mailboxEditorValue: 'flow-batch@mail-tw.707079.xyz',
                },
              ],
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
        search: '?mode=batchOauth',
        state: {
          draft: {
            batchOauth: {
              rows: [
                {
                  id: 'row-1',
                  displayName: 'Batch Pending Mailbox Attach',
                  groupName: 'production',
                  mailboxInput: 'pending-batch@mail-tw.707079.xyz',
                  mailboxEditorOpen: true,
                  mailboxEditorValue: 'pending-batch@mail-tw.707079.xyz',
                  mailboxBusyAction: 'attach',
                },
              ],
            },
          },
        },
      }}
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(within(document.body).getByRole('button', { name: /submit mailbox/i })).toBeDisabled()
    await expect(canvas.getAllByRole('button', { name: /^generate$/i })[0]).toBeDisabled()
  },
}

export const MailboxGenerateFlow: Story = {
  render: () => (
    <AccountPoolStoryRouter
      initialEntry={{
        pathname: '/account-pool/upstream-accounts/new',
        search: '?mode=batchOauth',
        state: {
          draft: {
            batchOauth: {
              rows: [
                {
                  id: 'row-1',
                  displayName: 'Batch Generate Mailbox Flow',
                  groupName: 'production',
                },
              ],
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
        search: '?mode=batchOauth',
        state: {
          draft: {
            batchOauth: {
              rows: [
                {
                  id: 'row-1',
                  displayName: 'Batch Pending Mailbox Generate',
                  groupName: 'production',
                  mailboxInput: 'pending-generate@mail-tw.707079.xyz',
                  mailboxBusyAction: 'generate',
                },
              ],
            },
          },
        },
      }}
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const generateButton = canvas.getAllByRole('button', { name: /^generate$/i })[0]
    const mailboxChipButton = canvas.getByRole('button', { name: /copy mailbox/i })

    await expect(generateButton).toBeDisabled()
    await expect(generateButton.querySelector('.animate-spin')).not.toBeNull()
    await expect(mailboxChipButton).toBeDisabled()
  },
}

export const GroupNoteDraft: Story = {
  render: () => <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts/new?mode=batchOauth" />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const doc = canvasElement.ownerDocument
    const trigger = canvas.getAllByRole('combobox')[0]
    await userEvent.click(trigger)

    const searchInput = doc.body.querySelector('[cmdk-input]')
    if (!(searchInput instanceof HTMLInputElement)) {
      throw new Error('missing group combobox search input')
    }
    await userEvent.type(searchInput, 'new-team')

    const createOption = Array.from(doc.body.querySelectorAll('[cmdk-item]')).find((candidate) =>
      (candidate.textContent || '').toLowerCase().includes('new-team'),
    )
    if (!(createOption instanceof HTMLElement)) {
      throw new Error('missing create option for new-team')
    }
    await userEvent.click(createOption)

    const documentScope = within(doc.body)
    await userEvent.click(
      await documentScope.findByRole('button', {
        name: /编辑分组备注|edit group note/i,
      }),
    )
    await expect(
      documentScope.getByRole('dialog', { name: /编辑分组备注|edit group note/i }),
    ).toBeInTheDocument()
    await expect(documentScope.getByText(/new-team/i)).toBeInTheDocument()
  },
}

export const ActionTooltips: Story = {
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

export const MailboxReady: Story = {
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

export const InvitedBadgeTooltip: Story = {
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

export const MailboxExpired: Story = {
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

export const MailboxRefreshFailed: Story = {
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

export const MailboxDetachedName: Story = {
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

export const NameConflict: Story = {
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

export const DuplicateWarning: Story = {
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

export const MixedStates: Story = {
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
