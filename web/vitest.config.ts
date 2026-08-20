import path from "node:path";
import { fileURLToPath } from "node:url";
import { storybookTest } from "@storybook/addon-vitest/vitest-plugin";
import { playwright } from "@vitest/browser-playwright";
import { defineConfig, mergeConfig } from "vitest/config";

import { createAppViteConfig } from "./vite.config";

const dirname =
  typeof __dirname !== "undefined" ? __dirname : path.dirname(fileURLToPath(import.meta.url));
const browserApiPort = Number.parseInt(process.env.VITEST_BROWSER_API_PORT ?? "", 10);
const resolvedBrowserApiPort =
  Number.isSafeInteger(browserApiPort) && browserApiPort > 0 ? browserApiPort : 63315;

export default mergeConfig(
  createAppViteConfig("test"),
  defineConfig({
    test: {
      projects: [
        {
          extends: true,
          test: {
            name: "unit",
            include: ["src/**/*.{test,spec}.{ts,tsx}"],
          },
        },
        {
          extends: true,
          plugins: [
            storybookTest({
              configDir: path.join(dirname, ".storybook"),
              storybookScript: "bun run storybook:ci",
            }),
          ],
          test: {
            name: "storybook",
            browser: {
              enabled: true,
              api: resolvedBrowserApiPort,
              // Storybook test startup now scans a much larger story graph after mainline merges.
              // Keep browser-mode coverage stable by allowing a longer initial connection window.
              connectTimeout: 180_000,
              headless: true,
              provider: playwright({}),
              instances: [{ browser: "chromium" }],
            },
            setupFiles: ["./.storybook/vitest.setup.ts"],
          },
        },
      ],
    },
  }),
);
