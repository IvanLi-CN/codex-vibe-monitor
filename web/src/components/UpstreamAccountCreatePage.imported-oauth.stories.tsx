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

function renderImportedOauthStory(defaultGroupName = 'production') {
  return (
    <AccountPoolStoryRouter
      initialEntry={{
        pathname: '/account-pool/upstream-accounts/new',
        search: '?mode=import',
        state: {
          draft: {
            import: {
              defaultGroupName,
            },
          },
        },
      }}
    />
  )
}

function buildPastedCredential(
  overrides?: Partial<{
    email: string
    account_id: string
    expired: string
    _storybookStatus: string
    _storybookDetail: string
  }>,
) {
  return JSON.stringify({
    type: 'codex',
    email: overrides?.email ?? 'paste-story@duckmail.sbs',
    account_id: overrides?.account_id ?? 'acct_paste_story',
    expired: overrides?.expired ?? '2026-03-20T00:00:00.000Z',
    access_token: 'access-token',
    refresh_token: 'refresh-token',
    id_token: 'header.payload.signature',
    ...(overrides?._storybookStatus
      ? { _storybookStatus: overrides._storybookStatus }
      : {}),
    ...(overrides?._storybookDetail
      ? { _storybookDetail: overrides._storybookDetail }
      : {}),
  })
}

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
  render: () => renderImportedOauthStory(),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await uploadImportFixture(canvasElement)
    await expect(canvas.getByRole('button', { name: /validate/i })).toBeEnabled()
  },
}

export const IdlePasteEditor: Story = {
  render: () => renderImportedOauthStory(),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const editor = canvas.getByLabelText(/paste one credential json/i)
    await expect(editor).toHaveValue('')
    await expect(
      canvas.getByText(/paste exactly one credential json object/i),
    ).toBeInTheDocument()
  },
}

export const PasteInvalidEditable: Story = {
  render: () => renderImportedOauthStory(),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const editor = canvas.getByLabelText(/paste one credential json/i)
    await userEvent.click(editor)
    await userEvent.paste('[{"type":"codex"}]')
    await expect(
      canvas.getByText(/paste exactly one credential json object/i),
    ).toBeInTheDocument()
    await expect(editor).toHaveValue('[{"type":"codex"}]')
  },
}

export const PasteAddedToQueue: Story = {
  render: () => renderImportedOauthStory(),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const editor = canvas.getByLabelText(/paste one credential json/i)
    await userEvent.click(editor)
    await userEvent.paste(buildPastedCredential())
    await expect(canvas.getByText(/pasted credential #1\.json/i)).toBeInTheDocument()
    await expect(editor).toHaveValue('')
  },
}

export const BlockedByUnselectableGroupProxy: Story = {
  render: () => renderImportedOauthStory('staging'),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await uploadImportFixture(canvasElement)
    await expect(
      canvas.getByText(/group "staging" does not have any selectable bound proxy nodes\./i),
    ).toBeInTheDocument()
    await expect(canvas.getByRole('button', { name: /validate/i })).toBeDisabled()
  },
}
