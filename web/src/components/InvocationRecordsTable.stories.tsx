import { useEffect, useMemo, useRef, type ReactNode } from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { I18nProvider } from '../i18n'
import type { ApiPoolUpstreamRequestAttempt } from '../lib/api'
import { InvocationRecordsTable } from './InvocationRecordsTable'
import {
  createStoryPoolAttemptsByInvokeId,
  STORYBOOK_FIRST_RESPONSE_BYTE_SEMANTICS_RECORDS,
  STORYBOOK_INVOCATION_RECORDS,
} from './invocationRecordsStoryFixtures'

type PoolAttemptsByInvokeId = Record<string, ApiPoolUpstreamRequestAttempt[]>

type StorybookPoolAttemptsRegistry = {
  originalFetch: typeof window.fetch
  providers: Map<symbol, () => PoolAttemptsByInvokeId>
}

declare global {
  interface Window {
    __storybookPoolAttemptsRegistry__?: StorybookPoolAttemptsRegistry
  }
}

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

function ensureStorybookPoolAttemptsRegistry() {
  if (typeof window === 'undefined') return null

  const existingRegistry = window.__storybookPoolAttemptsRegistry__
  if (existingRegistry) return existingRegistry

  const originalFetch = window.fetch.bind(window)
  const providers = new Map<symbol, () => PoolAttemptsByInvokeId>()

  const mockedFetch: typeof window.fetch = async (input, init) => {
    const requestUrl = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url
    const url = new URL(requestUrl, window.location.origin)
    const poolAttemptsMatch = url.pathname.match(/^\/api\/invocations\/([^/]+)\/pool-attempts$/)

    if (poolAttemptsMatch) {
      const invokeId = decodeURIComponent(poolAttemptsMatch[1] ?? '')
      const providerGetters = Array.from(providers.values()).reverse()

      for (const getAttemptsByInvokeId of providerGetters) {
        const attempts = getAttemptsByInvokeId()[invokeId]
        if (attempts) {
          return jsonResponse(attempts)
        }
      }

      return jsonResponse([])
    }

    return originalFetch(input, init)
  }

  window.fetch = mockedFetch
  window.__storybookPoolAttemptsRegistry__ = {
    originalFetch,
    providers,
  }

  return window.__storybookPoolAttemptsRegistry__
}

function StorybookPoolAttemptsMock({ children, records }: { children: ReactNode; records: typeof STORYBOOK_INVOCATION_RECORDS }) {
  const poolAttemptsByInvokeId = useMemo(() => createStoryPoolAttemptsByInvokeId(records), [records])
  const poolAttemptsByInvokeIdRef = useRef(poolAttemptsByInvokeId)
  const providerIdRef = useRef<symbol>(Symbol('storybook-pool-attempts'))

  poolAttemptsByInvokeIdRef.current = poolAttemptsByInvokeId

  useEffect(() => {
    const registry = ensureStorybookPoolAttemptsRegistry()
    if (!registry) return

    registry.providers.set(providerIdRef.current, () => poolAttemptsByInvokeIdRef.current)

    return () => {
      const activeRegistry = window.__storybookPoolAttemptsRegistry__
      if (!activeRegistry) return

      activeRegistry.providers.delete(providerIdRef.current)
      if (activeRegistry.providers.size === 0) {
        window.fetch = activeRegistry.originalFetch
        delete window.__storybookPoolAttemptsRegistry__
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
    (Story, context) => (
      <I18nProvider>
        <StorybookPoolAttemptsMock
          records={(context.args.records as typeof STORYBOOK_INVOCATION_RECORDS | undefined) ?? STORYBOOK_INVOCATION_RECORDS}
        >
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

export const FirstResponseByteSemantics: Story = {
  args: {
    focus: 'network',
    records: STORYBOOK_FIRST_RESPONSE_BYTE_SEMANTICS_RECORDS,
    isLoading: false,
    error: null,
  },
  parameters: {
    docs: {
      description: {
        story:
          'Focused network view for the new first-response-byte-total semantics. The first row deliberately keeps `上游首字节 = 0.0 ms` while the cumulative `首字总耗时` stays near `9.36 s`, matching the user-facing clarification in the monitoring table.',
      },
    },
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
