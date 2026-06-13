import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'

const storybookModelOptionFiles = [
  'src/components/TagRuleDialog.stories.tsx',
  'src/components/UpstreamAccountsPage.story-helpers-runtime.tsx',
]

describe('storybook available model options', () => {
  it('does not advertise non-preset models in routing policy stories', () => {
    for (const relativePath of storybookModelOptionFiles) {
      const content = readFileSync(join(process.cwd(), relativePath), 'utf8')
      expect(content, relativePath).not.toContain("'o3'")
      expect(content, relativePath).not.toContain('"o3"')
      expect(content, relativePath).not.toContain('gpt-5.5-2026-05-01')
    }
  })
})
