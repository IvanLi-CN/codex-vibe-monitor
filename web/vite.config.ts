import "vitest/config";
import { readdirSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import react from "@vitejs/plugin-react";
import { defineConfig, loadEnv, type UserConfig } from "vite";
import { VitePWA } from "vite-plugin-pwa";

function normalizeBase(base: string | undefined): string {
  const raw = base?.trim() || "/";
  if (raw === "/") return raw;
  return `${raw.startsWith("/") ? raw : `/${raw}`}${raw.endsWith("/") ? "" : "/"}`;
}

function findInstallIconAsset(prefix: string, extension: ".png" | ".svg"): string {
  const candidates = readdirSync(resolve("public")).filter((name) => {
    if (!name.startsWith(`${prefix}-`) || !name.endsWith(extension)) return false;
    const digest = name.slice(prefix.length + 1, -extension.length);
    return digest.length === 12 && /^[0-9a-f]{12}$/.test(digest);
  });
  if (candidates.length !== 1) {
    throw new Error(
      `Expected one content-hashed ${prefix}${extension} asset, found ${candidates.length}: ${candidates.join(", ")}`,
    );
  }
  return candidates[0];
}

const installIconAssets = {
  favicon: findInstallIconAsset("favicon", ".svg"),
  icon192: findInstallIconAsset("icon-192", ".png"),
  icon512: findInstallIconAsset("icon-512", ".png"),
  maskable192: findInstallIconAsset("maskable-192", ".png"),
  maskable512: findInstallIconAsset("maskable-512", ".png"),
};
const pwaPrecacheIgnores = [
  "**/site.webmanifest",
  "**/version.json",
  ...Object.values(installIconAssets).map((filename) => `**/${filename}`),
];

type PwaManifestEntry = string | { url: string };
type PwaPluginWithApi = {
  api?: {
    extendManifestEntries: (
      callback: (entries: PwaManifestEntry[]) => PwaManifestEntry[] | undefined,
    ) => void;
  };
};

function isPwaRuntimeMetadataEntry(entry: PwaManifestEntry): boolean {
  const path = (typeof entry === "string" ? entry : entry.url).split(/[?#]/, 1)[0];
  const filename = path.slice(path.lastIndexOf("/") + 1);
  return filename === "site.webmanifest" || filename === "version.json";
}

function isPwaInstallIconEntry(entry: PwaManifestEntry): boolean {
  const path = (typeof entry === "string" ? entry : entry.url).split(/[?#]/, 1)[0];
  const filename = path.slice(path.lastIndexOf("/") + 1);
  return /^(?:favicon|icon-192|icon-512|maskable-192|maskable-512)-[0-9a-f]{12}\.(?:png|svg)$/.test(
    filename,
  );
}

function createPwaPlugins() {
  const pwaPlugins = VitePWA({
    injectRegister: false,
    registerType: "prompt",
    strategies: "injectManifest",
    srcDir: "src/pwa",
    filename: "sw.ts",
    manifestFilename: "site.webmanifest",
    includeAssets: ["brand-mark.svg", "social-preview.png"],
    includeManifestIcons: false,
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
          icons: [{ src: installIconAssets.icon192, sizes: "192x192", type: "image/png" }],
        },
        {
          name: "Live",
          short_name: "Live",
          url: "./#/live",
          icons: [{ src: installIconAssets.icon192, sizes: "192x192", type: "image/png" }],
        },
        {
          name: "Records",
          short_name: "Records",
          url: "./#/records",
          icons: [{ src: installIconAssets.icon192, sizes: "192x192", type: "image/png" }],
        },
      ],
      screenshots: [
        {
          src: "social-preview.png",
          sizes: "1774x887",
          type: "image/png",
          form_factor: "wide",
          label: "Codex Vibe Monitor dashboard preview",
        },
      ],
      icons: [
        {
          src: installIconAssets.icon192,
          sizes: "192x192",
          type: "image/png",
          purpose: "any",
        },
        {
          src: installIconAssets.icon512,
          sizes: "512x512",
          type: "image/png",
          purpose: "any",
        },
        {
          src: installIconAssets.favicon,
          sizes: "any",
          type: "image/svg+xml",
          purpose: "any",
        },
        {
          src: installIconAssets.maskable192,
          sizes: "192x192",
          type: "image/png",
          purpose: "maskable",
        },
        {
          src: installIconAssets.maskable512,
          sizes: "512x512",
          type: "image/png",
          purpose: "maskable",
        },
      ],
    },
    injectManifest: {
      globPatterns: ["**/*.{js,css,html,ico,png,svg,json}"],
      globIgnores: pwaPrecacheIgnores,
      maximumFileSizeToCacheInBytes: 4 * 1024 * 1024,
    },
    devOptions: {
      navigateFallback: "index.html",
    },
  });
  const pwaMainPlugin = pwaPlugins.find(({ name }) => name === "vite-plugin-pwa") as
    | PwaPluginWithApi
    | undefined;
  pwaPlugins.push({
    name: "codex-vibe-monitor-pwa-precache-contract",
    enforce: "post",
    buildStart() {
      // vite-plugin-pwa adds the manifest automatically; it must remain network-revalidated.
      pwaMainPlugin?.api?.extendManifestEntries((entries) =>
        entries.filter(
          (entry) => !isPwaRuntimeMetadataEntry(entry) && !isPwaInstallIconEntry(entry),
        ),
      );
    },
  });
  return pwaPlugins;
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
      {
        name: "codex-vibe-monitor-install-icon-assets",
        transformIndexHtml(html) {
          return html.replaceAll("%INSTALL_FAVICON%", installIconAssets.favicon);
        },
      },
      react(),
      !isStorybook && createPwaPlugins(),
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
