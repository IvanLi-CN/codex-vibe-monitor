const defaultPort = '30000'
const host = process.env.STORYBOOK_HOST || '127.0.0.1'
const port = process.env.STORYBOOK_PORT || defaultPort

if (port === '6006') {
  console.error('Port 6006 is reserved for other worktrees; choose a different STORYBOOK_PORT.')
  process.exit(1)
}

const child = Bun.spawn(['bunx', '--bun', 'storybook', 'dev', '--host', host, '--port', port], {
  stdin: 'inherit',
  stdout: 'inherit',
  stderr: 'inherit',
  env: process.env,
})

process.exit(await child.exited)
