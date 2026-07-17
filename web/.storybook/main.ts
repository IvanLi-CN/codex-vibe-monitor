import type { StorybookConfig } from "@storybook/react-vite";
import { mergeConfig, type PluginOption, type UserConfig } from "vite";
import { createAppViteConfig } from "../vite.config.ts";

function flattenPlugins(plugins: PluginOption[] | undefined): PluginOption[] {
  if (!plugins) return [];
  return plugins.flatMap((plugin) => (Array.isArray(plugin) ? flattenPlugins(plugin) : [plugin]));
}

function removePwaPlugins(config: UserConfig): UserConfig {
  return {
    ...config,
    plugins: flattenPlugins(config.plugins).filter((plugin) => {
      if (!plugin || typeof plugin !== "object" || !("name" in plugin)) return true;
      return !String(plugin.name).includes("vite-plugin-pwa");
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
  viteFinal: async (baseConfig) =>
    mergeConfig(removePwaPlugins(baseConfig), removePwaPlugins(createAppViteConfig("storybook"))),
};

export default config;
