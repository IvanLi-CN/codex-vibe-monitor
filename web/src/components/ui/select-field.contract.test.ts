import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join, relative } from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'

const projectRoot = fileURLToPath(new URL('../../../', import.meta.url))
const sourceRoot = join(projectRoot, 'src')
const allowedLowLevelImportFiles = new Set([
  'components/ui/select.tsx',
  'components/ui/select-field.tsx',
])

function walkTsxFiles(root: string): string[] {
  return readdirSync(root).flatMap((entry) => {
    const nextPath = join(root, entry)
    const stats = statSync(nextPath)
    if (stats.isDirectory()) {
      return walkTsxFiles(nextPath)
    }
    if (!nextPath.endsWith('.tsx') || nextPath.endsWith('.test.tsx')) {
      return []
    }
    return [nextPath]
  })
}

describe('SelectField source contract', () => {
  it('blocks native select usage outside tests', () => {
    const offenders = walkTsxFiles(sourceRoot)
      .map((filePath) => ({
        relativePath: relative(sourceRoot, filePath).replaceAll('\\', '/'),
        content: readFileSync(filePath, 'utf8'),
      }))
      .filter(({ content }) => content.includes('<select'))
      .map(({ relativePath }) => relativePath)

    expect(offenders).toEqual([])
  })

  it('blocks page and story imports of low-level select primitives', () => {
    const offenders = walkTsxFiles(sourceRoot)
      .map((filePath) => ({
        relativePath: relative(sourceRoot, filePath).replaceAll('\\', '/'),
        content: readFileSync(filePath, 'utf8'),
      }))
      .filter(({ relativePath }) => !allowedLowLevelImportFiles.has(relativePath))
      .filter(({ content }) =>
        content.includes("components/ui/select'") ||
        content.includes('components/ui/select"') ||
        content.includes("./select'") ||
        content.includes('./select"'),
      )
      .map(({ relativePath }) => relativePath)

    expect(offenders).toEqual([])
  })
})
