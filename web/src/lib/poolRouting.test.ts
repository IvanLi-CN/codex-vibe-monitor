import { describe, expect, it } from 'vitest'
import { generatePoolRoutingKey } from './poolRouting'

describe('generatePoolRoutingKey', () => {
  it('returns a cvm-prefixed 128-bit lowercase hex token', () => {
    const key = generatePoolRoutingKey()

    expect(key).toMatch(/^cvm-[0-9a-f]{32}$/)
    expect(key).toHaveLength(36)
  })

  it('produces distinct keys across repeated calls', () => {
    const generated = new Set(Array.from({ length: 4 }, () => generatePoolRoutingKey()))

    expect(generated.size).toBe(4)
  })
})
