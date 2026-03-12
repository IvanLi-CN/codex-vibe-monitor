import type { Meta, StoryObj } from '@storybook/react-vite'
import { MemoryRouter, Route, Routes } from 'react-router-dom'
import { I18nProvider } from '../i18n'
import AccountPoolLayout from '../pages/account-pool/AccountPoolLayout'

function MockModuleContent() {
  return (
    <section className="surface-panel overflow-hidden">
      <div className="surface-panel-body gap-4">
        <div className="section-heading">
          <span className="text-xs font-semibold uppercase tracking-[0.24em] text-primary/80">
            Upstream accounts
          </span>
          <h2 className="section-title text-xl">Module content slot</h2>
          <p className="section-description max-w-2xl">
            Preview content rendered inside the account-pool layout outlet.
          </p>
        </div>
        <div className="grid gap-3 md:grid-cols-2">
          <div className="rounded-2xl border border-base-300 bg-base-100/90 p-4">
            <p className="text-sm font-medium text-base-content">5h usage window</p>
            <p className="mt-2 text-3xl font-semibold text-primary">64%</p>
            <p className="mt-1 text-sm text-base-content/70">Primary quota snapshot in the layout context.</p>
          </div>
          <div className="rounded-2xl border border-base-300 bg-base-100/90 p-4">
            <p className="text-sm font-medium text-base-content">7d usage window</p>
            <p className="mt-2 text-3xl font-semibold text-primary">22%</p>
            <p className="mt-1 text-sm text-base-content/70">Secondary quota snapshot in the layout context.</p>
          </div>
        </div>
      </div>
    </section>
  )
}

const meta = {
  title: 'Modules/Account Pool/Layout/Module Layout',
  component: AccountPoolLayout,
  tags: ['autodocs'],
  decorators: [
    (Story) => (
      <I18nProvider>
        <MemoryRouter initialEntries={['/account-pool/upstream-accounts']}>
          <Routes>
            <Route path="/account-pool" element={<Story />}>
              <Route path="upstream-accounts" element={<MockModuleContent />} />
            </Route>
          </Routes>
        </MemoryRouter>
      </I18nProvider>
    ),
  ],
  parameters: {
    layout: 'fullscreen',
  },
} satisfies Meta<typeof AccountPoolLayout>

export default meta

type Story = StoryObj<typeof meta>

export const Default: Story = {}
