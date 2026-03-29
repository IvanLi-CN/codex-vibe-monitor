import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, within } from 'storybook/test'
import {
  AccountPoolStoryRouter,
  UpstreamAccountCreatePage,
  upstreamAccountCreateMetaBase,
} from './UpstreamAccountCreatePage.story-common'

const meta = {
  ...upstreamAccountCreateMetaBase,
  title: 'Account Pool/Pages/Upstream Account Create/Imported OAuth',
} satisfies Meta<typeof UpstreamAccountCreatePage>

export default meta

type Story = StoryObj<typeof meta>

async function uploadImportFixture(canvasElement: HTMLElement) {
  const canvas = within(canvasElement)
  const fileInput = canvasElement.querySelector('input[type="file"]')
  if (!(fileInput instanceof HTMLInputElement)) {
    throw new Error('missing imported oauth file input')
  }
  const file = new File(
    [
      JSON.stringify({
        type: 'codex',
        email: 'story-import@duckmail.sbs',
        account_id: 'acct_story_import',
        expired: '2026-03-20T00:00:00.000Z',
        access_token: 'access-token',
        refresh_token: 'refresh-token',
        id_token: 'header.payload.signature',
      }),
    ],
    'story-import@duckmail.sbs.json',
    { type: 'application/json' },
  )
  await userEvent.upload(fileInput, file)
  await expect(canvas.getByText(/story-import@duckmail\.sbs\.json/i)).toBeInTheDocument()
}

export const ReadyToValidate: Story = {
  render: () => (
    <AccountPoolStoryRouter
      initialEntry={{
        pathname: '/account-pool/upstream-accounts/new',
        search: '?mode=import',
        state: {
          draft: {
            import: {
              defaultGroupName: 'production',
            },
          },
        },
      }}
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await uploadImportFixture(canvasElement)
    await expect(canvas.getByRole('button', { name: /validate/i })).toBeEnabled()
  },
}

export const BlockedByUnselectableGroupProxy: Story = {
  render: () => (
    <AccountPoolStoryRouter
      initialEntry={{
        pathname: '/account-pool/upstream-accounts/new',
        search: '?mode=import',
        state: {
          draft: {
            import: {
              defaultGroupName: 'staging',
            },
          },
        },
      }}
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await uploadImportFixture(canvasElement)
    await expect(
      canvas.getByText(/group "staging" does not have any selectable bound proxy nodes\./i),
    ).toBeInTheDocument()
    await expect(canvas.getByRole('button', { name: /validate/i })).toBeDisabled()
  },
}
