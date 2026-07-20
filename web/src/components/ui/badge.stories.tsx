import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, within } from "storybook/test";
import { Badge } from "./badge";

function StorySurface({ children }: { children: React.ReactNode }) {
  return (
    <div className="min-h-screen bg-base-200 px-6 py-6 text-base-content sm:px-8">
      <div className="mx-auto max-w-5xl rounded-[1.75rem] border border-base-300/70 bg-base-100/88 p-6 shadow-sm">
        {children}
      </div>
    </div>
  );
}

function BadgeGallery() {
  const variants = [
    { label: "Default", variant: "default" as const, note: "Primary tone ink on a neutral card" },
    {
      label: "Accent",
      variant: "accent" as const,
      note: "Accent chip stays readable in dark mode",
    },
    { label: "Secondary", variant: "secondary" as const, note: "Neutral support metadata" },
    { label: "Success", variant: "success" as const, note: "Passed, healthy, or completed state" },
    { label: "Info", variant: "info" as const, note: "Informational inline status" },
    {
      label: "Warning",
      variant: "warning" as const,
      note: "Caution without reusing filled-content ink",
    },
    {
      label: "Error",
      variant: "error" as const,
      note: "Failure state on low-opacity semantic fill",
    },
  ];

  return (
    <div className="space-y-5">
      <div className="space-y-2">
        <p className="text-xs font-semibold uppercase tracking-[0.24em] text-base-content/56">
          Semantic Tone Ink
        </p>
        <h2 className="text-2xl font-semibold text-base-content">
          Low-opacity semantic badges stay on tone ink, not filled-content.
        </h2>
        <p className="max-w-3xl text-sm leading-6 text-base-content/72">
          This gallery is the shared regression surface for dark and light neutral cards, so the
          accent and warning variants do not fall back to unreadable filled-content tokens again.
        </p>
      </div>

      <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
        {variants.map((item) => (
          <div
            key={item.variant}
            className="rounded-[1.2rem] border border-base-300/70 bg-base-100/72 p-4 shadow-sm"
          >
            <div className="flex items-center justify-between gap-3">
              <span className="text-sm font-medium text-base-content/84">{item.label}</span>
              <Badge variant={item.variant}>{item.label}</Badge>
            </div>
            <p className="mt-3 text-sm leading-6 text-base-content/72">{item.note}</p>
          </div>
        ))}
      </div>
    </div>
  );
}

const meta = {
  title: "UI/Badge",
  component: Badge,
  tags: ["autodocs"],
  decorators: [
    (Story) => (
      <StorySurface>
        <Story />
      </StorySurface>
    ),
  ],
} satisfies Meta<typeof Badge>;

export default meta;

type Story = StoryObj<typeof meta>;

export const SemanticToneGallery: Story = {
  render: () => <BadgeGallery />,
};

export const SemanticToneGalleryDark: Story = {
  ...SemanticToneGallery,
  globals: {
    themeMode: "dark",
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText("Accent")).toBeVisible();
    await expect(canvas.getByText("Warning")).toBeVisible();
    await expect(
      canvas.getByText(/Low-opacity semantic badges stay on tone ink, not filled-content\./),
    ).toBeVisible();
  },
};
