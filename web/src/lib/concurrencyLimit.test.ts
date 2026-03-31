import { describe, expect, it } from 'vitest'
import {
  apiConcurrencyLimitToSliderValue,
  sliderConcurrencyLimitToApiValue,
} from './concurrencyLimit'

describe('concurrencyLimit helpers', () => {
  it('maps unlimited storage values to the final slider step', () => {
    expect(apiConcurrencyLimitToSliderValue(0)).toBe(31)
    expect(apiConcurrencyLimitToSliderValue(undefined)).toBe(31)
  })

  it('maps the final slider step back to unlimited storage', () => {
    expect(sliderConcurrencyLimitToApiValue(31)).toBe(0)
  })

  it('preserves finite limits in both directions', () => {
    expect(apiConcurrencyLimitToSliderValue(6)).toBe(6)
    expect(sliderConcurrencyLimitToApiValue(6)).toBe(6)
  })
})
