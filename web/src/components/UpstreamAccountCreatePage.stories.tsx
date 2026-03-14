import type { Meta, StoryObj } from '@storybook/react-vite'
import { userEvent, within, expect } from 'storybook/test'
import { I18nProvider } from '../i18n'
import UpstreamAccountCreatePage from '../pages/account-pool/UpstreamAccountCreate'
import {
  AccountPoolStoryRouter,
  StorybookUpstreamAccountsMock,
  createPendingSession,
} from './UpstreamAccountsPage.story-helpers'

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
        <StorybookUpstreamAccountsMock>
          <Story />
        </StorybookUpstreamAccountsMock>
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

export const BatchOauthReady: Story = {
  render: () => <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts/new?mode=batchOauth" />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(canvas.getByRole('button', { name: /generate oauth url/i }))
    await expect(canvas.getByDisplayValue(/https:\/\/auth\.openai\.com\/authorize/i)).toBeInTheDocument()
    await expect(canvas.getByRole('button', { name: /complete oauth login/i })).toBeInTheDocument()
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
