import { spawn } from 'node:child_process'

const defaultPort = '30000'
const host = process.env.STORYBOOK_HOST || '127.0.0.1'
const port = process.env.STORYBOOK_PORT || defaultPort

if (port === '6006') {
  console.error('Port 6006 is reserved for other worktrees; choose a different STORYBOOK_PORT.')
  process.exit(1)
}

const npx = process.platform === 'win32' ? 'npx.cmd' : 'npx'
const child = spawn(npx, ['storybook', 'dev', '--host', host, '--port', port], {
  stdio: 'inherit',
  env: process.env,
})

child.on('exit', (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal)
    return
  }
  process.exit(code ?? 0)
})
