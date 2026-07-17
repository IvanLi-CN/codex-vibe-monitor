import "vitest/config";
import { fileURLToPath } from "node:url";
import react from "@vitejs/plugin-react";
import { defineConfig, loadEnv, type UserConfig } from "vite";
import { VitePWA } from "vite-plugin-pwa";

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
  const base = normalizeBase(env.VITE_DEPLOY_BASE);
  const isStorybook = mode === "storybook";

  if (runtime !== "live" && runtime !== "demo") {
    throw new Error(`Unsupported VITE_APP_RUNTIME: ${runtime}`);
  }

  const port = Number(env.VITE_APP_PORT ?? (demo ? "60083" : "60080"));
  if (!Number.isInteger(port) || port < 1 || port > 65535) {
    throw new Error(`Invalid VITE_APP_PORT: ${env.VITE_APP_PORT}`);
  }

  return {
    base,
    plugins: [
      react(),
      !isStorybook &&
        VitePWA({
          injectRegister: false,
          registerType: "prompt",
          strategies: "injectManifest",
          srcDir: "src/pwa",
          filename: "sw.ts",
          manifestFilename: "site.webmanifest",
          includeAssets: [
            "apple-touch-icon.png",
            "brand-mark.svg",
            "favicon.svg",
            "icon-192.png",
            "icon-512.png",
            "social-preview.png",
          ],
          manifest: {
            id: "./",
            name: "Codex Vibe Monitor",
            short_name: "Vibe Monitor",
            description:
              "Self-hosted observability workspace for OpenAI-compatible proxy traffic, request records, routing, and upstream account pools.",
            theme_color: "#0ea5e9",
            background_color: "#0ea5e9",
            display: "standalone",
            display_override: ["window-controls-overlay", "standalone"],
            start_url: "./#/dashboard",
            scope: "./",
            orientation: "any",
            categories: ["developer tools", "productivity", "utilities"],
            shortcuts: [
              {
                name: "Dashboard",
                short_name: "Dashboard",
                url: "./#/dashboard",
                icons: [{ src: "icon-192.png", sizes: "192x192", type: "image/png" }],
              },
              {
                name: "Live",
                short_name: "Live",
                url: "./#/live",
                icons: [{ src: "icon-192.png", sizes: "192x192", type: "image/png" }],
              },
              {
                name: "Records",
                short_name: "Records",
                url: "./#/records",
                icons: [{ src: "icon-192.png", sizes: "192x192", type: "image/png" }],
              },
            ],
            screenshots: [
              {
                src: "social-preview.png",
                sizes: "1200x630",
                type: "image/png",
                form_factor: "wide",
                label: "Codex Vibe Monitor dashboard preview",
              },
            ],
            icons: [
              {
                src: "icon-192.png",
                sizes: "192x192",
                type: "image/png",
                purpose: "any",
              },
              {
                src: "icon-512.png",
                sizes: "512x512",
                type: "image/png",
                purpose: "any",
              },
              {
                src: "favicon.svg",
                sizes: "any",
                type: "image/svg+xml",
                purpose: "any",
              },
            ],
          },
          injectManifest: {
            globPatterns: ["**/*.{js,css,html,ico,png,svg,webmanifest,json}"],
            maximumFileSizeToCacheInBytes: 4 * 1024 * 1024,
          },
          devOptions: {
            navigateFallback: "index.html",
          },
        }),
    ],
    resolve: isStorybook
      ? {
          alias: {
            "virtual:pwa-register": fileURLToPath(
              new URL("./src/pwa/storybook-register-sw.ts", import.meta.url),
            ),
          },
        }
      : undefined,
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
      include: ["@iconify-icons/mdi/compare-horizontal", "@iconify-icons/mdi/sort-variant"],
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
