import type { StorybookConfig } from "@storybook/react-vite";

const config: StorybookConfig = {
  stories: [
    "../src/components/**/*.stories.@(js|jsx|mjs|ts|tsx)",
    "../src/demo/**/*.stories.@(js|jsx|mjs|ts|tsx)",
    "../src/features/**/*.stories.@(js|jsx|mjs|ts|tsx)",
  ],
  addons: ["@storybook/addon-a11y", "@storybook/addon-docs", "@storybook/addon-vitest"],
  framework: "@storybook/react-vite",
};

export default config;
