import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, within } from "storybook/test";
import { ModelIdentity, ModelIdentityGroup } from "./ModelIdentity";

const meta = {
  title: "Components/ModelIdentity",
  component: ModelIdentity,
  tags: ["autodocs"],
  parameters: { layout: "centered" },
  decorators: [
    (Story) => (
      <div className="w-[min(32rem,calc(100vw-2rem))] rounded-xl border border-base-300 bg-base-100 p-6 text-base-content shadow-sm">
        <div className="flex flex-wrap items-center gap-5 text-sm">
          <Story />
        </div>
      </div>
    ),
  ],
  args: { model: "gpt-5.6-sol" },
} satisfies Meta<typeof ModelIdentity>;

export default meta;
type Story = StoryObj<typeof meta>;

export const SolTerraLuna: Story = {
  render: () => (
    <>
      <ModelIdentity model="gpt-5.6-sol" testId="model-sol" />
      <ModelIdentity model="gpt-5.6-terra" testId="model-terra" />
      <ModelIdentity model="gpt-5.6-luna" testId="model-luna" />
    </>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByTestId("model-sol")).toHaveAttribute("aria-label", "gpt-5.6-sol");
    await expect(canvas.getByTestId("model-terra")).toHaveAttribute("data-model-icon", "earth");
    await expect(canvas.getByTestId("model-luna")).toHaveAttribute("title", "gpt-5.6-luna");
  },
};

export const DatedVariantAndFallback: Story = {
  render: () => (
    <>
      <ModelIdentity model="gpt-5.6-sol-2026-07-08" testId="model-dated" />
      <ModelIdentity model="gpt-5.5" testId="model-fallback" />
    </>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByTestId("model-dated")).toHaveAttribute(
      "data-model-icon",
      "white-balance-sunny",
    );
    await expect(canvas.getByTestId("model-fallback")).toHaveTextContent("gpt-5.5");
  },
};

export const CompleteFamilyGroup: Story = {
  render: () => (
    <div className="flex min-w-0 flex-wrap items-center gap-3">
      <ModelIdentityGroup
        models={["gpt-5.6-luna", "gpt-5.6-sol", "gpt-5.6-terra"]}
        testId="model-family-group"
      />
      <ModelIdentityGroup
        models={["gpt-5.6-sol-2026-07-08", "gpt-5.6-terra-2026-07-08", "gpt-5.6-luna-2026-07-08"]}
        testId="model-family-dated-group"
      />
    </div>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByTestId("model-family-group")).toHaveAttribute(
      "aria-label",
      "gpt-5.6-sol · gpt-5.6-terra · gpt-5.6-luna",
    );
    await expect(canvas.getByTestId("model-family-dated-group")).toHaveAttribute(
      "data-model-identity-group",
      "gpt-5.6",
    );
  },
};
