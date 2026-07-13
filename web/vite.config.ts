import "vitest/config";
import react from "@vitejs/plugin-react";
import { defineConfig, loadEnv, type UserConfig } from "vite";

function normalizeBase(base: string | undefined): string {
  const raw = base?.trim() || "/";
  if (raw === "/") return raw;
  return `${raw.startsWith("/") ? raw : `/${raw}`}${raw.endsWith("/") ? "" : "/"}`;
}

export function createAppViteConfig(mode: string): UserConfig {
  const env = loadEnv(mode, process.cwd(), "VITE_");
  const backend = env.VITE_BACKEND_PROXY ?? "http://localhost:8080";
  const runtime = env.VITE_APP_RUNTIME ?? "live";
  const demo = runtime === "demo";

  if (runtime !== "live" && runtime !== "demo") {
    throw new Error(`Unsupported VITE_APP_RUNTIME: ${runtime}`);
  }

  const port = Number(env.VITE_APP_PORT ?? (demo ? "60083" : "60080"));
  if (!Number.isInteger(port) || port < 1 || port > 65535) {
    throw new Error(`Invalid VITE_APP_PORT: ${env.VITE_APP_PORT}`);
  }

  return {
    base: normalizeBase(env.VITE_DEPLOY_BASE),
    plugins: [react()],
    build: demo
      ? {
          outDir: env.VITE_BUILD_OUT_DIR ?? "demo-dist",
          emptyOutDir: true,
        }
      : undefined,
    test: {
      setupFiles: "./src/test-setup.ts",
      maxWorkers: 4,
      testTimeout: 20_000,
      hookTimeout: 20_000,
    },
    optimizeDeps: {
      include: ["@iconify-icons/mdi/compare-horizontal"],
    },
    server: {
      host: "127.0.0.1",
      port,
      strictPort: true,
      proxy: {
        "/api": {
          target: backend,
          changeOrigin: true,
        },
        "/events": {
          target: backend,
          changeOrigin: true,
        },
      },
    },
    preview: {
      host: "127.0.0.1",
      port,
      strictPort: true,
    },
  };
}

export default defineConfig(({ mode }) => createAppViteConfig(mode));
