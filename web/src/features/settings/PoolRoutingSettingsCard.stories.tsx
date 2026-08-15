import type { Meta, StoryObj } from "@storybook/react-vite";
import { useState } from "react";
import { expect, userEvent, within } from "storybook/test";
import { I18nProvider } from "../../i18n";
import type { AvailableModelsMode } from "../../lib/api";
import { PoolRoutingSettingsCard } from "./PoolRoutingSettingsCard";

const meta = {
  title: "Settings/Components/Pool Routing Settings Card",
  component: PoolRoutingSettingsCard,
  tags: ["autodocs"],
  parameters: {
    layout: "fullscreen",
    docs: {
      description: {
        component: "Root routing defaults with the layered model allowlist and denylist controls.",
      },
    },
  },
  args: {
    draft: {
      requestCompressionAlgorithm: "zstd",
      requestCompressionLevelPreset: "best",
      codexImagegenRewriteMode: "keep_original",
      availableModels: ["gpt-image-2", "gpt-5.4-mini"],
      availableModelsMode: "allowlist",
      responsesFirstByteTimeoutSecs: "120",
      compactFirstByteTimeoutSecs: "300",
      imageFirstByteTimeoutSecs: "300",
      responsesStreamTimeoutSecs: "300",
      compactStreamTimeoutSecs: "300",
      cacheHitProtectionEnabled: false,
      cacheHitRateThresholdPercent: "10",
      cacheHitOverflowMode: "queue",
    },
    busy: false,
    writesEnabled: true,
    canSave: false,
    onAlgorithmChange: () => undefined,
    onLevelPresetChange: () => undefined,
    onCodexImagegenRewriteModeChange: () => undefined,
    availableModelOptions: ["gpt-5.6-sol", "gpt-5.4-mini", "gpt-image-2", "gpt-image-1"],
    onAvailableModelsChange: () => undefined,
    onAvailableModelsModeChange: () => undefined,
    onTimeoutChange: () => undefined,
    onCacheHitProtectionChange: () => undefined,
    onSave: () => undefined,
  },
  decorators: [
    (Story) => (
      <I18nProvider>
        <div className="min-h-screen bg-base-200 px-[30px] pb-[9.5px] pt-[10.5px] text-base-content sm:px-10">
          <div className="mx-auto max-w-4xl">
            <Story />
          </div>
        </div>
      </I18nProvider>
    ),
  ],
} satisfies Meta<typeof PoolRoutingSettingsCard>;

export default meta;

type Story = StoryObj<typeof meta>;

export const ModelPolicy: Story = {
  render: () => {
    const [availableModelsMode, setAvailableModelsMode] =
      useState<AvailableModelsMode>("allowlist");
    const [availableModels, setAvailableModels] = useState(["gpt-image-2", "gpt-5.4-mini"]);

    return (
      <PoolRoutingSettingsCard
        draft={{
          requestCompressionAlgorithm: "zstd",
          requestCompressionLevelPreset: "best",
          codexImagegenRewriteMode: "keep_original",
          availableModels,
          availableModelsMode,
          responsesFirstByteTimeoutSecs: "120",
          compactFirstByteTimeoutSecs: "300",
          imageFirstByteTimeoutSecs: "300",
          responsesStreamTimeoutSecs: "300",
          compactStreamTimeoutSecs: "300",
          cacheHitProtectionEnabled: false,
          cacheHitRateThresholdPercent: "10",
          cacheHitOverflowMode: "queue",
        }}
        busy={false}
        writesEnabled
        canSave={false}
        onAlgorithmChange={() => undefined}
        onLevelPresetChange={() => undefined}
        onCodexImagegenRewriteModeChange={() => undefined}
        availableModelOptions={["gpt-5.6-sol", "gpt-5.4-mini", "gpt-image-2", "gpt-image-1"]}
        onAvailableModelsChange={setAvailableModels}
        onAvailableModelsModeChange={setAvailableModelsMode}
        onTimeoutChange={() => undefined}
        onCacheHitProtectionChange={() => undefined}
        onSave={() => undefined}
      />
    );
  },
};

export const CacheHitProtection: Story = {
  args: {
    draft: {
      requestCompressionAlgorithm: "zstd",
      requestCompressionLevelPreset: "best",
      codexImagegenRewriteMode: "keep_original",
      availableModels: ["gpt-image-2", "gpt-5.4-mini"],
      availableModelsMode: "allowlist",
      responsesFirstByteTimeoutSecs: "120",
      compactFirstByteTimeoutSecs: "300",
      imageFirstByteTimeoutSecs: "300",
      responsesStreamTimeoutSecs: "300",
      compactStreamTimeoutSecs: "300",
      cacheHitProtectionEnabled: true,
      cacheHitRateThresholdPercent: "10",
      cacheHitOverflowMode: "reroute",
    },
  },
  tags: ["test"],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(
      canvas.getByRole("switch", { name: /缓存命中保护|Cache-hit protection/ }),
    ).toHaveAttribute("aria-checked", "true");
    await expect(canvas.getByDisplayValue("10")).toBeEnabled();
  },
};

export const DesktopModeToggle: Story = {
  ...ModelPolicy,
  tags: ["test"],
  parameters: {
    viewport: {
      defaultViewport: "desktop1280",
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const toggle = canvas.getByRole("button", {
      name: /切换模型策略模式|Switch model policy mode/,
    });

    await expect(toggle).toHaveAttribute("aria-pressed", "true");
    await userEvent.click(toggle);
    await expect(toggle).toHaveAttribute("aria-pressed", "false");
    await expect(toggle).toHaveTextContent(/黑名单|Denylist/);
  },
};
