import { readFileSync, writeFileSync, mkdirSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const __filename = fileURLToPath(import.meta.url)
const __dirname = dirname(__filename)

try {
  const pkg = JSON.parse(readFileSync(join(__dirname, '..', 'package.json'), 'utf8'))
  const version = process.env.VITE_APP_VERSION || pkg.version || '0.0.0'
  const distDir = join(__dirname, '..', 'dist')
  mkdirSync(distDir, { recursive: true })
  writeFileSync(join(distDir, 'version.json'), JSON.stringify({ version }, null, 2) + '\n', 'utf8')
  console.log(`[version] wrote ${version} to dist/version.json`)
} catch (err) {
  console.error('[version] failed to write version.json:', err)
  process.exit(1)
}

