import { useEffect, useMemo, useRef, type ReactNode } from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { I18nProvider } from '../i18n'
import { InvocationRecordsTable } from './InvocationRecordsTable'
import { createStoryPoolAttemptsByInvokeId, STORYBOOK_INVOCATION_RECORDS } from './invocationRecordsStoryFixtures'

function StorySurface({ children }: { children: React.ReactNode }) {
  return (
    <div className="min-h-screen bg-base-200 px-6 py-6 text-base-content">
      <div className="mx-auto w-full max-w-[1440px]">{children}</div>
    </div>
  )
}

function jsonResponse(payload: unknown) {
  return new Response(JSON.stringify(payload), {
    status: 200,
    headers: {
      'Content-Type': 'application/json',
    },
  })
}

function StorybookPoolAttemptsMock({ children, records }: { children: ReactNode; records: typeof STORYBOOK_INVOCATION_RECORDS }) {
  const originalFetchRef = useRef<typeof window.fetch | null>(null)
  const poolAttemptsByInvokeId = useMemo(() => createStoryPoolAttemptsByInvokeId(records), [records])

  if (typeof window !== 'undefined' && !originalFetchRef.current) {
    originalFetchRef.current = window.fetch.bind(window)

    const mockedFetch: typeof window.fetch = async (input, init) => {
      const requestUrl = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url
      const url = new URL(requestUrl, window.location.origin)
      const poolAttemptsMatch = url.pathname.match(/^\/api\/invocations\/([^/]+)\/pool-attempts$/)

      if (poolAttemptsMatch) {
        const invokeId = decodeURIComponent(poolAttemptsMatch[1] ?? '')
        return jsonResponse(poolAttemptsByInvokeId[invokeId] ?? [])
      }

      return (originalFetchRef.current as typeof window.fetch)(input, init)
    }

    window.fetch = mockedFetch
  }

  useEffect(() => {
    return () => {
      if (typeof window !== 'undefined' && originalFetchRef.current) {
        window.fetch = originalFetchRef.current
        originalFetchRef.current = null
      }
    }
  }, [])

  return <>{children}</>
}

const meta = {
  title: 'Records/InvocationRecordsTable',
  component: InvocationRecordsTable,
  parameters: {
    layout: 'fullscreen',
  },
  decorators: [
    (Story) => (
      <I18nProvider>
        <StorybookPoolAttemptsMock records={STORYBOOK_INVOCATION_RECORDS}>
          <StorySurface>
            <Story />
          </StorySurface>
        </StorybookPoolAttemptsMock>
      </I18nProvider>
    ),
  ],
} satisfies Meta<typeof InvocationRecordsTable>

export default meta

type Story = StoryObj<typeof meta>

export const TokenFocus: Story = {
  args: {
    focus: 'token',
    records: STORYBOOK_INVOCATION_RECORDS,
    isLoading: false,
    error: null,
  },
}

export const NetworkFocus: Story = {
  args: {
    focus: 'network',
    records: STORYBOOK_INVOCATION_RECORDS,
    isLoading: false,
    error: null,
  },
}

export const ExceptionFocus: Story = {
  args: {
    focus: 'exception',
    records: STORYBOOK_INVOCATION_RECORDS,
    isLoading: false,
    error: null,
  },
}

export const StructuredOnlyFocus: Story = {
  args: {
    focus: 'exception',
    records: STORYBOOK_INVOCATION_RECORDS.filter((record) => record.detailLevel === 'structured_only'),
    isLoading: false,
    error: null,
  },
}

export const PoolRouteFocus: Story = {
  args: {
    focus: 'network',
    records: STORYBOOK_INVOCATION_RECORDS.filter((record) => record.routeMode === 'pool'),
    isLoading: false,
    error: null,
  },
}

export const Loading: Story = {
  args: {
    focus: 'token',
    records: [],
    isLoading: true,
    error: null,
  },
}

export const Empty: Story = {
  args: {
    focus: 'token',
    records: [],
    isLoading: false,
    error: null,
  },
}
