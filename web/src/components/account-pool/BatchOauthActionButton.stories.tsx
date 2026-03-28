import type { Meta, StoryObj } from "@storybook/react-vite";
import type { ComponentProps, ReactNode } from "react";
import { expect, userEvent, within } from "storybook/test";
import { BatchOauthActionButton } from "./BatchOauthActionButton";

function StorySurface({ children }: { children: ReactNode }) {
  return (
    <div className="min-h-screen bg-base-200 px-10 py-12">
      <div className="max-w-xl rounded-2xl border border-base-300/80 bg-base-100 p-6 shadow-sm">
        <div className="flex items-center gap-3">
          <span className="field-label shrink-0">OAuth flow</span>
          {children}
        </div>
      </div>
    </div>
  );
}

const meta = {
  title: "Account Pool/Pages/Upstream Account Create/Batch OAuth Action",
  component: BatchOauthActionButton,
  tags: ["autodocs"],
  decorators: [
    (Story) => (
      <StorySurface>
        <Story />
      </StorySurface>
    ),
  ],
  parameters: {
    layout: "fullscreen",
  },
} satisfies Meta<typeof BatchOauthActionButton>;

export default meta;

type Story = StoryObj<typeof meta>;

const baseArgs = {
  primaryAriaLabel: "Copy OAuth URL",
  regenerateAriaLabel: "Regenerate OAuth URL",
  popoverTitle: "Copy OAuth URL",
  popoverDescription:
    "Copy the generated login URL, open it in the browser that will complete the login, and return here with the callback URL.",
  remainingLabel: "Current link expires in 14:59.",
  expiresAtLabel: "Expires at 2026-03-26 18:00:00.",
  manualCopyTitle: "Copy manually",
  manualCopyDescription: "Clipboard access failed. Copy the latest OAuth URL below instead.",
  onPrimaryAction: () => undefined,
  onRegenerate: () => undefined,
} satisfies Partial<ComponentProps<typeof BatchOauthActionButton>>;

export const Generate: Story = {
  args: {
    ...baseArgs,
    mode: "generate",
    primaryAriaLabel: "Generate OAuth URL",
    popoverTitle: "Generate OAuth URL",
    popoverDescription:
      "Confirm the metadata for this row, then generate the login URL and continue from that same URL.",
    remainingLabel: null,
    expiresAtLabel: null,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const button = canvas.getByRole("button", { name: /generate oauth url/i });

    await userEvent.hover(button);
    await new Promise((resolve) => window.setTimeout(resolve, 330));
    await expect(
      within(document.body).getByText(/generate oauth url/i),
    ).toBeInTheDocument();
  },
};

export const CopyPopover: Story = {
  args: {
    ...baseArgs,
    mode: "copy",
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const button = canvas.getByRole("button", { name: /copy oauth url/i });

    button.dispatchEvent(
      new MouseEvent("contextmenu", {
        bubbles: true,
        cancelable: true,
      }),
    );

    const popover = within(document.body);
    await expect(
      popover.getByText(/current link expires in 14:59/i),
    ).toBeInTheDocument();
    await expect(
      popover.getByRole("button", { name: /regenerate oauth url/i }),
    ).toBeInTheDocument();
  },
};

export const ManualFallback: Story = {
  args: {
    ...baseArgs,
    mode: "copy",
    manualCopyValue: "https://auth.openai.com/authorize?login=manual",
  },
};

export const LongPress: Story = {
  args: {
    ...baseArgs,
    mode: "copy",
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const button = canvas.getByRole("button", { name: /copy oauth url/i });

    button.dispatchEvent(
      new PointerEvent("pointerdown", {
        bubbles: true,
        pointerType: "touch",
        button: 0,
      }),
    );
    await new Promise((resolve) => window.setTimeout(resolve, 430));

    await expect(
      within(document.body).getByText(/current link expires in 14:59/i),
    ).toBeInTheDocument();
  },
};
