import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, within } from 'storybook/test'
import { I18nProvider } from '../i18n'
import {
  ImportedOauthValidationDialog,
  type ImportedOauthValidationDialogState,
} from './ImportedOauthValidationDialog'

const mixedResultsState: ImportedOauthValidationDialogState = {
  inputFiles: 6,
  uniqueInInput: 5,
  duplicateInInput: 1,
  checking: false,
  importing: false,
  rows: [
    {
      sourceId: 'tokyo',
      fileName: 'tokyo@duckmail.sbs.json',
      email: 'tokyo@duckmail.sbs',
      chatgptAccountId: 'acct_tokyo',
      displayName: 'tokyo@duckmail.sbs',
      tokenExpiresAt: '2026-03-19T10:30:00.000Z',
      status: 'ok',
      detail: null,
      attempts: 1,
      matchedAccount: null,
    },
    {
      sourceId: 'seoul',
      fileName: 'seoul@duckmail.sbs.json',
      email: 'seoul@duckmail.sbs',
      chatgptAccountId: 'acct_seoul',
      displayName: 'seoul@duckmail.sbs',
      tokenExpiresAt: '2026-03-18T16:10:00.000Z',
      status: 'ok_exhausted',
      detail: 'Usage snapshot indicates the account is currently exhausted.',
      attempts: 1,
      matchedAccount: {
        accountId: 52,
        displayName: 'Seoul Prod',
        groupName: 'prod',
        status: 'active',
      },
    },
    {
      sourceId: 'broken',
      fileName: 'broken@duckmail.sbs.json',
      email: 'broken@duckmail.sbs',
      chatgptAccountId: 'acct_broken',
      displayName: 'broken@duckmail.sbs',
      tokenExpiresAt: '2026-03-18T15:00:00.000Z',
      status: 'invalid',
      detail: 'id_token subject does not match top-level email field.',
      attempts: 2,
      matchedAccount: null,
    },
    {
      sourceId: 'error',
      fileName: 'error@duckmail.sbs.json',
      email: 'error@duckmail.sbs',
      chatgptAccountId: 'acct_error',
      displayName: 'error@duckmail.sbs',
      tokenExpiresAt: '2026-03-18T15:00:00.000Z',
      status: 'error',
      detail: 'Network timeout while refreshing imported OAuth token.',
      attempts: 1,
      matchedAccount: null,
    },
    {
      sourceId: 'duplicate',
      fileName: 'duplicate@duckmail.sbs.json',
      email: 'tokyo@duckmail.sbs',
      chatgptAccountId: 'acct_tokyo',
      displayName: 'tokyo@duckmail.sbs',
      tokenExpiresAt: '2026-03-18T15:00:00.000Z',
      status: 'duplicate_in_input',
      detail: 'Duplicate credential in current import selection.',
      attempts: 0,
      matchedAccount: null,
    },
  ],
  importReport: null,
  importError: null,
}

const meta = {
  title: 'Account Pool/Components/Imported OAuth Validation Dialog',
  component: ImportedOauthValidationDialog,
  tags: ['autodocs'],
  parameters: {
    layout: 'fullscreen',
  },
  decorators: [
    (Story) => (
      <I18nProvider>
        <div className="min-h-screen bg-base-200 p-6 text-base-content">
          <Story />
        </div>
      </I18nProvider>
    ),
  ],
  args: {
    open: true,
    state: mixedResultsState,
    onClose: () => undefined,
    onRetryFailed: () => undefined,
    onRetryOne: () => undefined,
    onImportValid: () => undefined,
  },
} satisfies Meta<typeof ImportedOauthValidationDialog>

export default meta

type Story = StoryObj<typeof meta>

export const MixedResults: Story = {}

export const CheckingInProgress: Story = {
  args: {
    state: {
      ...mixedResultsState,
      checking: true,
      rows: mixedResultsState.rows.map((row) => ({
        ...row,
        status: 'pending',
        detail: null,
      })),
    },
  },
}

export const PostImportReport: Story = {
  args: {
    state: {
      ...mixedResultsState,
      importReport: {
        summary: {
          inputFiles: 6,
          selectedFiles: 2,
          created: 1,
          updatedExisting: 1,
          failed: 0,
        },
        results: [
          {
            sourceId: 'tokyo',
            fileName: 'tokyo@duckmail.sbs.json',
            email: 'tokyo@duckmail.sbs',
            chatgptAccountId: 'acct_tokyo',
            accountId: 88,
            status: 'created',
            detail: null,
            matchedAccount: null,
          },
          {
            sourceId: 'seoul',
            fileName: 'seoul@duckmail.sbs.json',
            email: 'seoul@duckmail.sbs',
            chatgptAccountId: 'acct_seoul',
            accountId: 52,
            status: 'updated_existing',
            detail: 'Imported, but initial sync failed: transient upstream timeout.',
            matchedAccount: {
              accountId: 52,
              displayName: 'Seoul Prod',
              groupName: 'prod',
              status: 'active',
            },
          },
        ],
      },
    },
  },
}

export const FilterToFailedRows: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(canvas.getByRole('button', { name: /^invalid/i }))
    await expect(canvas.getByText(/id_token subject does not match/i)).toBeInTheDocument()
    await expect(canvas.queryByText(/duplicate credential in current import selection/i)).toBeNull()
  },
}
