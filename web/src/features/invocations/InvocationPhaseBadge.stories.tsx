import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, within } from "storybook/test";
import { I18nProvider } from "../../i18n";
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
    <div className="flex flex-col gap-4">
      <div className="space-y-2">
        <div className="text-[11px] font-semibold uppercase tracking-[0.18em] text-base-content/52">
          Record dynamic
        </div>
        <div className="flex flex-wrap items-center gap-3">
          <InvocationPhaseBadge phase="queued" motion="dynamic" />
          <InvocationPhaseBadge phase="requesting" motion="dynamic" />
          <InvocationPhaseBadge phase="responding" motion="dynamic" />
        </div>
      </div>
      <div className="space-y-2">
        <div className="text-[11px] font-semibold uppercase tracking-[0.18em] text-base-content/52">
          Summary static
        </div>
        <InvocationPhaseSegments
          counts={{ queued: 1, requesting: 1, responding: 1 }}
          appearance="inline"
          motion="static"
        />
      </div>
    </div>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const badges = await canvas.findAllByTestId("invocation-phase-badge");
    expect(badges).toHaveLength(3);
    const icons = await canvas.findAllByTestId("invocation-phase-icon");
    expect(icons[1]?.getAttribute("data-phase-icon-name")).toBe(
      "navigation-variant",
    );
    expect(icons[1]?.className).toContain(
      "animate-invocation-phase-requesting",
    );
    expect(icons[2]?.className).toContain("animate-spin");
    expect(icons[2]?.getAttribute("data-phase-icon-name")).toBe("sync");
    expect(icons[4]?.getAttribute("data-phase-icon-name")).toBe(
      "navigation-variant",
    );
    expect(icons[5]?.getAttribute("data-phase-icon-name")).toBe(
      "chat-processing-outline",
    );
    expect(icons[4]?.className).not.toContain(
      "animate-invocation-phase-requesting",
    );
    expect(icons[5]?.className).not.toContain("animate-spin");
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
      expect(icon.className).not.toContain(
        "animate-invocation-phase-requesting",
      );
      expect(icon.className).not.toContain("animate-pulse");
      expect(icon.className).not.toContain("animate-spin");
    }
    expect(icons[1]?.getAttribute("data-phase-icon-name")).toBe(
      "navigation-variant",
    );
    expect(icons[2]?.getAttribute("data-phase-icon-name")).toBe(
      "chat-processing-outline",
    );
  },
};
