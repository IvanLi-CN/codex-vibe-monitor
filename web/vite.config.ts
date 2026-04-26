import 'vitest/config'
import { defineConfig, loadEnv, type UserConfig } from 'vite'
import react from '@vitejs/plugin-react'

export function createAppViteConfig(mode: string): UserConfig {
  const env = loadEnv(mode, process.cwd(), 'VITE_')
  const backend = env.VITE_BACKEND_PROXY ?? 'http://localhost:8080'

  return {
    plugins: [react()],
    test: {
      setupFiles: './src/test-setup.ts',
      maxWorkers: 4,
      testTimeout: 20_000,
      hookTimeout: 20_000,
    },
    server: {
      host: '127.0.0.1',
      port: 60080,
      strictPort: true,
      proxy: {
        '/api': {
          target: backend,
          changeOrigin: true,
        },
        '/events': {
          target: backend,
          changeOrigin: true,
        },
      },
    },
  }
}

export default defineConfig(({ mode }) => createAppViteConfig(mode))
