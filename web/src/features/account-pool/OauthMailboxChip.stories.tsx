import type { Meta, StoryObj } from "@storybook/react-vite";
import { act, type ComponentProps, type ReactNode } from "react";
import { expect, userEvent, waitFor, within } from "storybook/test";
import { OauthMailboxChip } from "./OauthMailboxChip";

function StorySurface({ children }: { children: ReactNode }) {
  return (
    <div data-visual-evidence-surface className="min-h-screen bg-base-200 px-10 py-12">
      <div
        data-visual-evidence-target
        className="max-w-xl rounded-2xl border border-base-300/80 bg-base-100 p-6 shadow-sm"
      >
        <div className="flex items-center gap-3">
          <span className="field-label shrink-0">Display Name</span>
          {children}
        </div>
      </div>
    </div>
  );
}

const meta = {
  title: "Account Pool/Pages/Upstream Account Create/Mailbox Chip",
  component: OauthMailboxChip,
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
} satisfies Meta<typeof OauthMailboxChip>;

export default meta;

type Story = StoryObj<typeof meta>;

async function waitForTransfer(milliseconds: number) {
  await act(async () => {
    await new Promise((resolve) => window.setTimeout(resolve, milliseconds));
  });
}

const baseArgs = {
  className: "max-w-[24rem]",
  emptyLabel: "No mailbox yet",
  copyAriaLabel: "Copy mailbox",
  copyHintLabel: "Click to copy",
  copiedLabel: "Copied",
  manualCopyLabel: "Auto copy failed. Please copy the mailbox below manually.",
  manualBadgeLabel: "Manual",
  onCopy: () => undefined,
} satisfies Partial<ComponentProps<typeof OauthMailboxChip>>;

export const Hover: Story = {
  tags: ["test"],
  args: {
    ...baseArgs,
    emailAddress: "hover-preview@mail-tw.707079.xyz",
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const copyMailboxButton = canvas.getByRole("button", { name: /copy mailbox/i });

    await userEvent.hover(copyMailboxButton);
    const popup = await waitFor(() => {
      const visiblePopup = document.body.querySelector<HTMLElement>(
        '[data-side][data-state]:not([data-state="closed"])',
      );
      if (!visiblePopup) throw new Error("Tooltip content has not opened");
      return visiblePopup;
    });
    expect(popup).toHaveAttribute("data-side");
    await userEvent.unhover(copyMailboxButton);
    await waitForTransfer(300);
    const visibleContent = popup.querySelector<HTMLElement>("div.flex");
    if (!visibleContent) throw new Error("Tooltip visible content has not rendered");
    await expect(within(visibleContent).getByText(/click to copy/i)).toBeInTheDocument();
    await userEvent.hover(popup);
    await waitForTransfer(300);
    await expect(within(visibleContent).getByText(/click to copy/i)).toBeInTheDocument();
    await expect(
      within(visibleContent).getByText(/hover-preview@mail-tw\.707079\.xyz/i),
    ).toBeInTheDocument();
  },
};

export const LongPress: Story = {
  args: {
    ...baseArgs,
    emailAddress: "press-preview@mail-tw.707079.xyz",
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const copyMailboxButton = canvas.getByRole("button", { name: /copy mailbox/i });

    copyMailboxButton.dispatchEvent(
      new PointerEvent("pointerdown", { bubbles: true, pointerType: "touch", button: 0 }),
    );
    await waitForTransfer(420);

    const tooltip = within(document.body);
    await expect(tooltip.getByText(/click to copy/i)).toBeInTheDocument();
    await expect(tooltip.getByText(/press-preview@mail-tw\.707079\.xyz/i)).toBeInTheDocument();

    copyMailboxButton.dispatchEvent(
      new PointerEvent("pointerup", { bubbles: true, pointerType: "touch", button: 0 }),
    );
  },
};

export const Copied: Story = {
  args: {
    ...baseArgs,
    emailAddress: "copied-preview@mail-tw.707079.xyz",
    tone: "copied",
  },
};

export const ManualCopy: Story = {
  args: {
    ...baseArgs,
    emailAddress: "manual-copy@mail-tw.707079.xyz",
    tone: "manual",
  },
};

export const EditablePopover: Story = {
  tags: ["test"],
  args: {
    ...baseArgs,
    emailAddress: "editable-preview@mail-tw.707079.xyz",
    editor: {
      draftValue: "editable-preview@mail-tw.707079.xyz",
      inputAriaLabel: "Mailbox address",
      inputPlaceholder: "mailbox@example.com",
      editAriaLabel: "Edit mailbox",
      editHintLabel: "Unused helper copy",
      submitAriaLabel: "Submit mailbox",
      cancelAriaLabel: "Cancel mailbox edit",
      startEditing: () => undefined,
      onDraftValueChange: () => undefined,
      onSubmit: () => undefined,
      onCancel: () => undefined,
      editing: false,
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const copyMailboxButton = canvas.getByRole("button", { name: /copy mailbox/i });

    await userEvent.hover(copyMailboxButton);

    const popover = within(document.body);
    const editButton = await popover.findByRole("button", { name: /edit mailbox/i });
    const panel = editButton.closest("[data-side]");
    expect(panel).not.toBeNull();
    await userEvent.unhover(copyMailboxButton);
    await waitForTransfer(300);
    await expect(within(panel as HTMLElement).getByRole("button", { name: /edit mailbox/i })).toBe(
      editButton,
    );
    await userEvent.hover(panel as HTMLElement);
    await waitForTransfer(300);
    await expect(within(panel as HTMLElement).getByText(/click to copy/i)).toBeInTheDocument();
  },
};

export const HoverNarrow: Story = {
  ...Hover,
  tags: ["test"],
  parameters: {
    viewport: { defaultViewport: "mobile390" },
  },
};

export const EditablePopoverNarrow: Story = {
  ...EditablePopover,
  tags: ["test"],
  parameters: {
    viewport: { defaultViewport: "mobile390" },
  },
};

export const EditableCopiedFeedback: Story = {
  args: {
    ...baseArgs,
    emailAddress: "copied-editor@mail-tw.707079.xyz",
    tone: "copied",
    editor: {
      draftValue: "copied-editor@mail-tw.707079.xyz",
      inputAriaLabel: "Mailbox address",
      inputPlaceholder: "mailbox@example.com",
      editAriaLabel: "Edit mailbox",
      editHintLabel: "Unused helper copy",
      submitAriaLabel: "Submit mailbox",
      cancelAriaLabel: "Cancel mailbox edit",
      startEditing: () => undefined,
      onDraftValueChange: () => undefined,
      onSubmit: () => undefined,
      onCancel: () => undefined,
      editing: false,
    },
  },
};
