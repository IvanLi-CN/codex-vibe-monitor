/** @vitest-environment jsdom */
import * as React from 'react'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it } from 'vitest'
import type { EffectiveRoutingRule } from '../lib/api'
import { EffectiveRoutingRuleCard } from './EffectiveRoutingRuleCard'

beforeAll(() => {
  Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
    configurable: true,
    writable: true,
    value: true,
  })
})

let root: Root | null = null
let host: HTMLDivElement | null = null

afterEach(() => {
  act(() => {
    root?.unmount()
  })
  host?.remove()
  root = null
  host = null
})

function render(ui: React.ReactNode) {
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
  act(() => {
    root?.render(ui)
  })
}

const labels = {
  title: 'Effective routing rule',
  description: 'Merged routing constraints applied to the selected upstream account.',
  noTags: 'No tags linked',
  blockNewConversations: 'Block new conversations',
  allowNewConversations: 'New conversations are not blocked',
  allowCutOut: 'Cut-out not blocked',
  denyCutOut: 'Cut-out blocked',
  allowCutIn: 'Cut-in not blocked',
  denyCutIn: 'Cut-in blocked',
  sourceTags: 'Source tags',
  priorityPrimary: 'Primary',
  priorityNormal: 'Normal',
  priorityFallback: 'Fallback only',
  fastModeKeepOriginal: 'Keep original',
  fastModeFillMissing: 'Fill when missing',
  fastModeForceAdd: 'Force add',
  fastModeForceRemove: 'Force remove',
  upstream429Retry: '429 retry enabled',
  upstream429RetryOff: '429 retry off',
  availableModelsInherited: 'Inherited / unrestricted',
  availableModelsNoneAllowed: 'No models allowed',
  systemDeniedModelsEmpty: 'None',
  concurrencyLimit: (count: number) => `Concurrency ${count}`,
  concurrencyUnlimited: 'Concurrency unlimited',
  sourceBreakdownTitle: 'Field source breakdown',
  fieldBlockNewConversations: 'Block new conversations',
  fieldAllowCutOut: 'Cut out',
  fieldAllowCutIn: 'Cut in',
  fieldPriority: 'Priority',
  fieldFastMode: 'FAST mode',
  fieldConcurrency: 'Concurrency',
  fieldUpstream429: 'Upstream 429 retry',
  fieldAvailableModels: 'Available models',
  fieldSystemDeniedModels: 'System denied models',
  sourceRoot: 'Root default',
  sourceGroup: 'Group',
  sourceTag: 'Tag',
  sourceAccount: 'Account',
  sourceSystem: 'System',
}

function buildRule(overrides: Partial<EffectiveRoutingRule> = {}): EffectiveRoutingRule {
  return {
    blockNewConversations: false,
    allowCutOut: true,
    allowCutIn: true,
    priorityTier: 'normal',
    fastModeRewriteMode: 'keep_original',
    concurrencyLimit: 0,
    upstream429RetryEnabled: false,
    upstream429MaxRetries: 0,
    availableModels: [],
    systemDeniedModels: [],
    sourceTagIds: [],
    sourceTagNames: [],
    fieldSources: {
      blockNewConversations: 'root',
      allowCutOut: 'root',
      allowCutIn: 'root',
      priorityTier: 'root',
      fastModeRewriteMode: 'root',
      concurrencyLimit: 'root',
      upstream429Retry: 'root',
      availableModels: 'root',
      systemDeniedModels: 'root',
    },
    ...overrides,
  }
}

describe('EffectiveRoutingRuleCard', () => {
  it('shows inherited copy when no available model constraint is defined', () => {
    render(
      <EffectiveRoutingRuleCard
        rule={buildRule()}
        labels={labels}
      />,
    )

    expect(document.body.textContent).toContain('Inherited / unrestricted')
    expect(document.body.textContent).not.toContain('No models allowed')
  })

  it('shows deny-all copy for empty tag intersections', () => {
    render(
      <EffectiveRoutingRuleCard
        rule={buildRule({
          availableModels: [],
          sourceTagIds: [1, 2],
          sourceTagNames: ['allow-gpt-4o', 'allow-o3'],
          fieldSources: {
            blockNewConversations: 'root',
            allowCutOut: 'root',
            allowCutIn: 'root',
            priorityTier: 'tag',
            fastModeRewriteMode: 'tag',
            concurrencyLimit: 'tag',
            upstream429Retry: 'root',
            availableModels: 'tag',
            systemDeniedModels: 'root',
          },
        })}
        labels={labels}
      />,
    )

    expect(document.body.textContent).toContain('No models allowed')
    expect(document.body.textContent).not.toContain('Inherited / unrestricted')
  })
})
