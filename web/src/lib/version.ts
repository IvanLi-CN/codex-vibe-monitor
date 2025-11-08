import pkg from '../../package.json' assert { type: 'json' }

const packageVersion = (pkg as { version?: string }).version

export const frontendVersion =
  import.meta.env.VITE_APP_VERSION ?? packageVersion ?? 'unknown'

export function normalizeVersion(version: string | null | undefined): string {
  if (!version) return 'unknown'
  const trimmed = version.trim()
  if (!trimmed) return 'unknown'
  if (/^[0-9]/.test(trimmed)) {
    return trimmed.startsWith('v') ? trimmed : `v${trimmed}`
  }
  return trimmed
}
