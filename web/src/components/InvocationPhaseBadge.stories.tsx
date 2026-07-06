import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, within } from "storybook/test";
import { I18nProvider } from "../i18n";
import {
  InvocationPhaseBadge,
  InvocationPhaseSegments,
} from "./InvocationPhaseBadge";

const meta = {
  title: "Components/InvocationPhaseBadge",
  component: InvocationPhaseBadge,
  tags: ["autodocs"],
  parameters: {
    layout: "centered",
  },
  decorators: [
    (Story) => (
      <I18nProvider>
        <div className="flex min-w-[22rem] justify-center rounded-2xl bg-base-200 p-6 text-base-content">
          <Story />
        </div>
      </I18nProvider>
    ),
  ],
  args: {
    phase: "responding",
    motion: "dynamic",
  },
} satisfies Meta<typeof InvocationPhaseBadge>;

export default meta;

type Story = StoryObj<typeof meta>;

export const RecordPhases: Story = {
  render: () => (
    <div className="flex flex-wrap items-center gap-3">
      <InvocationPhaseBadge phase="queued" motion="dynamic" />
      <InvocationPhaseBadge phase="requesting" motion="dynamic" />
      <InvocationPhaseBadge phase="responding" motion="dynamic" />
    </div>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const badges = await canvas.findAllByTestId("invocation-phase-badge");
    expect(badges).toHaveLength(3);
    const icons = await canvas.findAllByTestId("invocation-phase-icon");
    expect(icons[1]?.className).toContain("animate-pulse");
    expect(icons[2]?.className).toContain("animate-spin");
  },
};

export const CompactIcons: Story = {
  render: () => (
    <div className="flex items-center gap-3">
      <InvocationPhaseBadge
        phase="requesting"
        appearance="inline"
        motion="dynamic"
        showLabel={false}
      />
      <InvocationPhaseBadge
        phase="responding"
        appearance="inline"
        motion="dynamic"
        showLabel={false}
      />
    </div>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const badges = await canvas.findAllByTestId("invocation-phase-badge");
    expect(badges).toHaveLength(2);
    for (const badge of badges) {
      expect(badge.getAttribute("data-phase-label-visible")).toBe("false");
      expect(badge.className).toContain("rounded-full");
    }
  },
};

export const SummarySegments: Story = {
  render: () => (
    <InvocationPhaseSegments
      counts={{ queued: 2, requesting: 3, responding: 4 }}
      appearance="inline"
      motion="static"
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const segments = await canvas.findAllByTestId("invocation-phase-segment");
    expect(segments).toHaveLength(3);
    const icons = await canvas.findAllByTestId("invocation-phase-icon");
    for (const segment of segments) {
      expect(segment.getAttribute("data-phase-motion")).toBe("static");
    }
    for (const icon of icons) {
      expect(icon.className).not.toContain("animate-pulse");
      expect(icon.className).not.toContain("animate-spin");
    }
  },
};
