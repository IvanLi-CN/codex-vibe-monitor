import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, userEvent, within } from "storybook/test";
import { PwaInstallTrigger } from "./PwaInstallTrigger";

const meta = {
  title: "Shell/PWA Install Trigger",
  component: PwaInstallTrigger,
  tags: ["autodocs"],
  args: {
    mode: "prompt",
    label: "Install app",
    ariaLabel: "Open install app controls",
    onClick: () => undefined,
  },
  parameters: { layout: "centered" },
} satisfies Meta<typeof PwaInstallTrigger>;

export default meta;
type Story = StoryObj<typeof meta>;

export const DesktopPrompt: Story = {};

export const MobileSafari: Story = {
  args: { mode: "manual-ios", label: "Add to Home Screen", compact: true },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByRole("button", { name: "Open install app controls" }));
    await expect(canvas.getByRole("button")).toBeInTheDocument();
  },
};
