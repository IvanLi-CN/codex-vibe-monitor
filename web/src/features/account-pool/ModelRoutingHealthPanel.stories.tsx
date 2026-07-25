import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, userEvent, within } from "storybook/test";
import { I18nProvider } from "../../i18n";
import type { ModelRoutingState } from "../../lib/api";
import { ModelRoutingHealthPanel } from "./ModelRoutingHealthPanel";

const now = new Date("2026-07-24T08:00:00.000Z").toISOString();

const states: ModelRoutingState[] = [
  {
    model: "gpt-5.5",
    state: "available",
    priority: "normal",
    failureCount: 0,
    changedAt: now,
    lastSeenAt: now,
  },
  {
    model: "gpt-5.4-mini",
    state: "degraded",
    priority: "demoted",
    failureCount: 3,
    changedAt: now,
    lastSeenAt: now,
    lastFailureAt: now,
    lastFailureKind: "model_unavailable",
    lastFailureMessage: "The requested model is temporarily unavailable upstream.",
  },
  {
    model: "o4-mini",
    state: "cooling_down",
    priority: "excluded",
    failureCount: 5,
    changedAt: now,
    lastSeenAt: now,
    lastFailureAt: now,
    lastFailureKind: "model_quota",
    lastFailureMessage: "Model-specific quota exhausted.",
    cooldownUntil: new Date("2026-07-24T08:00:45.000Z").toISOString(),
  },
];

const meta = {
  title: "Account Pool/ModelRoutingHealthPanel",
  component: ModelRoutingHealthPanel,
  tags: ["autodocs"],
  parameters: {
    layout: "fullscreen",
    a11y: {
      options: { rules: { "color-contrast": { enabled: true } } },
      config: { rules: [{ id: "color-contrast", enabled: true }] },
    },
  },
  decorators: [
    (Story) => (
      <I18nProvider>
        <div className="bg-base-200 px-6 py-8 text-base-content">
          <div className="mx-auto max-w-[1440px]">
            <Story />
          </div>
        </div>
      </I18nProvider>
    ),
  ],
  args: { states, writesEnabled: true, onReset: fn() },
} satisfies Meta<typeof ModelRoutingHealthPanel>;

export default meta;
type Story = StoryObj<typeof meta>;

export const MixedStates: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getAllByText("失败摘要", { exact: true })).toHaveLength(2);
    await expect(canvasElement.querySelectorAll("dt")).toHaveLength(17);
    await expect(
      canvas.getByText("The requested model is temporarily unavailable upstream.", {
        exact: true,
      }),
    ).toBeVisible();
  },
};

export const Empty: Story = {
  args: { states: [] },
};

export const ResetCoolingModel: Story = {
  args: { states, writesEnabled: true, onReset: fn() },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByTestId("model-routing-reset-o4-mini"));
    await expect(args.onReset).toHaveBeenCalledWith("o4-mini");
  },
};

export const ReadOnly: Story = {
  args: { states, writesEnabled: false },
};

export const ErrorState: Story = {
  args: {
    states,
    error: "模型路由状态刷新失败，请稍后重试。",
  },
};
