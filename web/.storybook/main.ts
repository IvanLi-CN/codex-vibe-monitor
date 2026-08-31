import type { StorybookConfig } from "@storybook/react-vite";
import { mergeConfig, type PluginOption, type UserConfig } from "vite";
import { createAppViteConfig } from "../vite.config.ts";

const PWA_PLUGIN_NAME_MARKERS = ["vite-plugin-pwa", "codex-vibe-monitor-pwa-precache-contract"];

function flattenPlugins(plugins: PluginOption[] | undefined): PluginOption[] {
  if (!plugins) return [];
  return plugins.flatMap((plugin) => (Array.isArray(plugin) ? flattenPlugins(plugin) : [plugin]));
}

function dedupePluginsByName(plugins: PluginOption[] | undefined): PluginOption[] | undefined {
  const flattened = flattenPlugins(plugins);
  const seen = new Set<string>();
  const deduped: PluginOption[] = [];
  for (const plugin of flattened) {
    if (!plugin || typeof plugin !== "object" || !("name" in plugin)) {
      deduped.push(plugin);
      continue;
    }
    const pluginName = String(plugin.name);
    if (seen.has(pluginName)) continue;
    seen.add(pluginName);
    deduped.push(plugin);
  }
  return deduped;
}

function removePwaPlugins(config: UserConfig): UserConfig {
  return {
    ...config,
    plugins: flattenPlugins(config.plugins).filter((plugin) => {
      if (!plugin || typeof plugin !== "object" || !("name" in plugin)) return true;
      const pluginName = String(plugin.name);
      return !PWA_PLUGIN_NAME_MARKERS.some((marker) => pluginName.includes(marker));
    }),
  };
}

const config: StorybookConfig = {
  stories: [
    "../src/components/**/*.stories.@(js|jsx|mjs|ts|tsx)",
    "../src/demo/**/*.stories.@(js|jsx|mjs|ts|tsx)",
    "../src/features/**/*.stories.@(js|jsx|mjs|ts|tsx)",
  ],
  addons: ["@storybook/addon-a11y", "@storybook/addon-docs", "@storybook/addon-vitest"],
  framework: "@storybook/react-vite",
  viteFinal: async (baseConfig) => {
    const merged = mergeConfig(
      removePwaPlugins(baseConfig),
      removePwaPlugins(createAppViteConfig("storybook")),
    );
    return {
      ...merged,
      plugins: dedupePluginsByName(merged.plugins),
    };
  },
};

export default config;
