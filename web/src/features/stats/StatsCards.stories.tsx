import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, within } from "storybook/test";
import { I18nProvider } from "../../i18n";
import { StatsCards } from "./StatsCards";

const meta = {
  title: "Stats/StatsCards",
  component: StatsCards,
  tags: ["autodocs", "test"],
  decorators: [
    (Story) => (
      <I18nProvider>
        <div
          data-visual-evidence-surface
          className="min-h-48 bg-base-200 px-6 py-6 text-base-content"
        >
          <div data-visual-evidence-target className="mx-auto w-full max-w-4xl">
            <Story />
          </div>
        </div>
      </I18nProvider>
    ),
  ],
} satisfies Meta<typeof StatsCards>;

export default meta;

type Story = StoryObj<typeof meta>;

export const RecoveryPending: Story = {
  args: {
    stats: null,
    loading: false,
    error: "Request failed",
    errorCode: "summary_recovery_pending",
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByTestId("stats-cards-grid")).toBeVisible();
    await expect(canvas.getByTestId("stats-cards-error-overlay")).toBeVisible();
    await expect(canvas.getByRole("alert")).toHaveTextContent(/后台恢复|recovery/i);
    await expect(canvas.getAllByTestId(/stats-card-value-.*-loading/)).toHaveLength(5);
  },
};

export const RecoveryFailed: Story = {
  args: {
    stats: null,
    loading: false,
    error: "Request failed",
    errorCode: "summary_recovery_failed",
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByTestId("stats-cards-grid")).toBeVisible();
    await expect(canvas.getByTestId("stats-cards-error-overlay")).toBeVisible();
    await expect(canvas.getByRole("alert")).toHaveTextContent(/恢复失败|recovery/i);
    await expect(canvas.getAllByTestId(/stats-card-value-.*-loading/)).toHaveLength(5);
  },
};
