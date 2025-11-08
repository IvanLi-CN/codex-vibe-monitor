import { defineConfig, loadEnv } from 'vite'
import react from '@vitejs/plugin-react'

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), 'VITE_')
  const backend = env.VITE_BACKEND_PROXY ?? 'http://localhost:8080'

  return {
    plugins: [react()],
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
})
