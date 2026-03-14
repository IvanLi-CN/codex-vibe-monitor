import { readdirSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { AppIcon } from './AppIcon'

function walkTsxFiles(dir: string): string[] {
  const entries = readdirSync(dir, { withFileTypes: true })
  const files: string[] = []

  for (const entry of entries) {
    const fullPath = join(dir, entry.name)
    if (entry.isDirectory()) {
      files.push(...walkTsxFiles(fullPath))
      continue
    }

    if (!entry.isFile() || !entry.name.endsWith('.tsx')) continue
    if (entry.name.endsWith('.test.tsx') || entry.name.endsWith('.stories.tsx')) continue
    if (entry.name === 'AppIcon.tsx') continue
    files.push(fullPath)
  }

  return files
}

describe('AppIcon registry', () => {
  it('renders bundled icons without needing runtime string lookups', () => {
    expect(() =>
      renderToStaticMarkup(
        <>
          <AppIcon name="close" />
          <AppIcon name="refresh" />
          <AppIcon name="help-circle-outline" />
          <AppIcon name="lightning-bolt" />
          <AppIcon name="weather-night" />
        </>,
      ),
    ).not.toThrow()
  })

  it('keeps runtime code free from direct Iconify string fetch paths', () => {
    const runtimeFiles = [
      ...walkTsxFiles(join(process.cwd(), 'src/components')),
      ...walkTsxFiles(join(process.cwd(), 'src/pages')),
    ]

    for (const filePath of runtimeFiles) {
      const content = readFileSync(filePath, 'utf8')
      expect(content, `${filePath} should not import @iconify/react directly`).not.toContain('@iconify/react')
      expect(content, `${filePath} should not use mdi: runtime icon strings`).not.toContain('mdi:')
    }
  })
})
