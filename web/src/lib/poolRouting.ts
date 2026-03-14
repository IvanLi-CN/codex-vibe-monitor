export function generatePoolRoutingKey(): string {
  const bytes = new Uint8Array(16)
  globalThis.crypto.getRandomValues(bytes)
  const token = Array.from(bytes, (value) => value.toString(16).padStart(2, '0')).join('')
  return `cvm-${token}`
}
