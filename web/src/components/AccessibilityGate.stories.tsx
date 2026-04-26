import type { Meta, StoryObj } from '@storybook/react-vite'
import { Button } from './ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from './ui/card'
import { Input } from './ui/input'
import { SelectField } from './ui/select-field'

function AccessibilityGateFixture() {
  return (
    <main className="min-h-screen bg-base-100 p-8 text-base-content">
      <Card className="mx-auto max-w-2xl">
        <CardHeader>
          <CardTitle>Storybook accessibility gate fixture</CardTitle>
          <CardDescription>
            A stable opt-in Storybook surface used by CI to prove the axe integration fails on accessibility regressions.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-5">
          <section aria-labelledby="gate-form-title" className="space-y-3">
            <h2 id="gate-form-title" className="text-base font-semibold">
              Labeled controls
            </h2>
            <div className="grid gap-3 sm:grid-cols-2">
              <div className="space-y-1.5">
                <label htmlFor="gate-name" className="text-sm font-medium text-base-content">Display name</label>
                <Input id="gate-name" defaultValue="Koha monitor" />
              </div>
              <SelectField
                id="gate-release-channel"
                label="Release channel"
                value="stable"
                onValueChange={() => undefined}
                options={[
                  { value: 'stable', label: 'Stable' },
                  { value: 'rc', label: 'Release candidate' },
                ]}
              />
            </div>
          </section>

          <section aria-labelledby="gate-status-title" className="space-y-3">
            <h2 id="gate-status-title" className="text-base font-semibold">
              Status summary
            </h2>
            <dl className="grid gap-3 rounded-lg border border-base-300 bg-base-100 p-4 sm:grid-cols-2">
              <div>
                <dt className="text-sm font-medium text-base-content/75">CI gate</dt>
                <dd className="mt-1 text-sm text-base-content">Storybook Accessibility</dd>
              </div>
              <div>
                <dt className="text-sm font-medium text-base-content/75">Mode</dt>
                <dd className="mt-1 text-sm text-base-content">axe semantic checks</dd>
              </div>
            </dl>
          </section>

          <div className="flex flex-wrap gap-3">
            <Button type="button">Primary action</Button>
            <Button type="button" variant="outline">
              Secondary action
            </Button>
          </div>
        </CardContent>
      </Card>
    </main>
  )
}

const meta = {
  title: 'Quality Gates/Accessibility Gate',
  component: AccessibilityGateFixture,
  tags: ['test'],
  parameters: {
    layout: 'fullscreen',
  },
} satisfies Meta<typeof AccessibilityGateFixture>

export default meta

type Story = StoryObj<typeof meta>

export const SemanticFixture: Story = {}
