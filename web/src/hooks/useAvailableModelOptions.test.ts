import { describe, expect, it } from 'vitest'
import type { SettingsPayload } from '../lib/api'
import { extractAvailableModelOptions } from './useAvailableModelOptions'

function createSettingsPayload(overrides: Partial<SettingsPayload> = {}): SettingsPayload {
  return {
    proxy: {
      hijackEnabled: false,
      mergeUpstreamEnabled: false,
      fastModeRewriteMode: 'disabled',
      upstream429MaxRetries: 3,
      websocketEnabled: false,
      upstreamWebsocketDefaultEnabled: false,
      defaultHijackEnabled: false,
      models: ['gpt-5.5', 'gpt-5.5-pro', 'gpt-5.4', 'gpt-5.4-pro'],
      enabledModels: ['gpt-5.5', 'gpt-5.5-pro', 'gpt-5.4', 'gpt-5.4-pro'],
    },
    forwardProxy: {
      proxyUrls: [],
      subscriptionUrls: [],
      subscriptionUpdateIntervalSecs: 600,
      nodes: [],
    },
    pricing: {
      catalogVersion: 'test-pricing',
      entries: [
        {
          model: 'gpt-5',
          inputPer1m: 1,
          outputPer1m: 2,
          source: 'official',
        },
        {
          model: 'gpt-5-chat-latest',
          inputPer1m: 1,
          outputPer1m: 2,
          source: 'official',
        },
      ],
    },
    ...overrides,
  }
}

describe('extractAvailableModelOptions', () => {
  it('uses only proxy preset models and excludes pricing-only models', () => {
    expect(extractAvailableModelOptions(createSettingsPayload())).toEqual([
      'gpt-5.5',
      'gpt-5.5-pro',
      'gpt-5.4',
      'gpt-5.4-pro',
    ])
  })

  it('preserves proxy preset order while removing duplicates and blanks', () => {
    const settings = createSettingsPayload({
      proxy: {
        ...createSettingsPayload().proxy,
        models: ['gpt-5.5', ' gpt-5.4-mini ', 'gpt-5.5', '', 'gpt-5.4'],
      },
    })

    expect(extractAvailableModelOptions(settings)).toEqual([
      'gpt-5.5',
      'gpt-5.4-mini',
      'gpt-5.4',
    ])
  })

  it('returns an empty list before settings load', () => {
    expect(extractAvailableModelOptions(null)).toEqual([])
  })
})
