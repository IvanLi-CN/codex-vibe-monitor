/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import { FilterableCombobox } from './filterable-combobox'

let host: HTMLDivElement | null = null
let root: Root | null = null

beforeAll(() => {
  Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
    configurable: true,
    writable: true,
    value: true,
  })
})

afterEach(() => {
  act(() => {
    root?.unmount()
  })
  host?.remove()
  host = null
  root = null
  vi.restoreAllMocks()
})

function render(ui: React.ReactNode) {
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
  act(() => {
    root?.render(ui)
  })
}

function getComboboxInput() {
  const input = document.querySelector('input[role="combobox"]')
  if (!(input instanceof HTMLInputElement)) {
    throw new Error('missing combobox input')
  }
  return input
}

describe('FilterableCombobox', () => {
  it('disables browser native autocomplete hints by default', () => {
    render(
      <FilterableCombobox
        label="Model"
        name="model"
        value=""
        onValueChange={() => {}}
        options={['gpt-5.4', 'deepseek-v3.1']}
      />,
    )

    const input = getComboboxInput()
    expect(input.autocomplete).toBe('off')
    expect(input.getAttribute('autocorrect')).toBe('off')
    expect(input.getAttribute('autocapitalize')).toBe('none')
    expect(input.getAttribute('spellcheck')).toBe('false')
  })

  it('allows explicit autocomplete overrides when a caller needs them', () => {
    render(
      <FilterableCombobox
        label="Model"
        name="model"
        value=""
        onValueChange={() => {}}
        options={['gpt-5.4', 'deepseek-v3.1']}
        inputAutocompleteProps={{
          autoComplete: 'on',
          autoCorrect: 'on',
          autoCapitalize: 'sentences',
          spellCheck: true,
        }}
      />,
    )

    const input = getComboboxInput()
    expect(input.autocomplete).toBe('on')
    expect(input.getAttribute('autocorrect')).toBe('on')
    expect(input.getAttribute('autocapitalize')).toBe('sentences')
    expect(input.getAttribute('spellcheck')).toBe('true')
  })
})
