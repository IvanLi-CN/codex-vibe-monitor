import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, within } from "storybook/test";
import type { ChipTone } from "./chip";
import { Chip } from "./chip";

const SEMANTIC_TONES: Array<{ tone: ChipTone; note: string }> = [
  { tone: "neutral", note: "ordinary metadata" },
  { tone: "primary", note: "current or active work" },
  { tone: "secondary", note: "supporting metadata" },
  { tone: "accent", note: "independent emphasis" },
  { tone: "info", note: "remote or in-progress information" },
  { tone: "success", note: "healthy or completed state" },
  { tone: "warning", note: "pending or restricted state" },
  { tone: "error", note: "failed or unavailable state" },
];

const CATEGORICAL_TONES: Array<{ tone: ChipTone; note: string }> = [
  { tone: "sky", note: "identity slot" },
  { tone: "cyan", note: "image endpoint" },
  { tone: "blue", note: "responses endpoint" },
  { tone: "indigo", note: "identity slot" },
  { tone: "violet", note: "remote compact endpoint" },
  { tone: "fuchsia", note: "identity slot" },
  { tone: "teal", note: "chat endpoint" },
  { tone: "emerald", note: "image generation endpoint" },
  { tone: "amber", note: "image edit endpoint" },
  { tone: "orange", note: "compact endpoint" },
];

function StorySurface({ children }: { children: React.ReactNode }) {
  return (
    <div className="min-h-screen bg-base-200 px-6 py-6 text-base-content sm:px-8">
      <div className="mx-auto max-w-5xl rounded-[1.75rem] border border-base-300/70 bg-base-100/88 p-6 shadow-sm">
        {children}
      </div>
    </div>
  );
}

function ChipGallery() {
  const tones = [...SEMANTIC_TONES, ...CATEGORICAL_TONES];
  return (
    <div className="space-y-5" data-testid="tone-gallery">
      <div className="space-y-2">
        <p className="text-xs font-semibold uppercase tracking-[0.24em] text-base-content/56">
          Shared chip palette
        </p>
        <h2 className="text-2xl font-semibold text-base-content">18 explicit tone presets</h2>
        <p className="max-w-3xl text-sm leading-6 text-base-content/72">
          Every textual chip uses this component, with opaque surface, border, and hue-aware ink
          tokens in both themes.
        </p>
      </div>

      <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
        {tones.map((item) => (
          <div
            key={item.tone}
            className="rounded-[1.2rem] border border-base-300/70 bg-base-100/72 p-4 shadow-sm"
          >
            <div className="flex items-center justify-between gap-3">
              <span className="text-sm font-medium text-base-content/84">{item.tone}</span>
              <Chip tone={item.tone} data-testid={`chip-${item.tone}`}>
                {item.tone}
              </Chip>
            </div>
            <p className="mt-3 text-sm leading-6 text-base-content/72">{item.note}</p>
          </div>
        ))}
      </div>

      <div className="flex items-center gap-3 border-t border-base-300/70 pt-4">
        <span className="text-sm text-base-content/72">Keyboard focus</span>
        <Chip asChild tone="primary" size="header" data-testid="focus-indicator-chip">
          <button type="button">Focus sample</button>
        </Chip>
      </div>
    </div>
  );
}

function parseColor(value: string): [number, number, number] {
  const probe = document.createElement("canvas").getContext("2d");
  if (!probe) throw new Error("Canvas is required for color assertions");
  probe.fillStyle = value;
  const normalized = probe.fillStyle;
  const rgbMatch = normalized.match(/rgba?\((\d+)[,\s]+(\d+)[,\s]+(\d+)/);
  if (rgbMatch) return [Number(rgbMatch[1]), Number(rgbMatch[2]), Number(rgbMatch[3])];

  const oklchMatch = normalized.match(/oklch\(\s*([\d.]+)(%?)\s+([\d.]+)\s+([\d.]+)\s*\)/);
  if (!oklchMatch) throw new Error(`Unable to parse computed color: ${value}`);

  const lightness = Number(oklchMatch[1]) / (oklchMatch[2] === "%" ? 100 : 1);
  const chroma = Number(oklchMatch[3]);
  const hue = (Number(oklchMatch[4]) * Math.PI) / 180;
  const labA = chroma * Math.cos(hue);
  const labB = chroma * Math.sin(hue);
  const l = lightness + 0.3963377774 * labA + 0.2158037573 * labB;
  const m = lightness - 0.1055613458 * labA - 0.0638541728 * labB;
  const s = lightness - 0.0894841775 * labA - 1.291485548 * labB;
  const l3 = l ** 3;
  const m3 = m ** 3;
  const s3 = s ** 3;
  const linearRgb = [
    4.0767416621 * l3 - 3.3077115913 * m3 + 0.2309699292 * s3,
    -1.2684380046 * l3 + 2.6097574011 * m3 - 0.3413193965 * s3,
    -0.0041960863 * l3 - 0.7034186176 * m3 + 1.707614701 * s3,
  ];
  return linearRgb.map((channel) => {
    const encoded = channel <= 0.0031308 ? 12.92 * channel : 1.055 * channel ** (1 / 2.4) - 0.055;
    return Math.round(Math.min(1, Math.max(0, encoded)) * 255);
  }) as [number, number, number];
}

function luminance(channel: number) {
  const value = channel / 255;
  return value <= 0.03928 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
}

function contrastRatio(foreground: string, background: string) {
  const [fr, fg, fb] = parseColor(foreground);
  const [br, bg, bb] = parseColor(background);
  const foregroundLuminance =
    0.2126 * luminance(fr) + 0.7152 * luminance(fg) + 0.0722 * luminance(fb);
  const backgroundLuminance =
    0.2126 * luminance(br) + 0.7152 * luminance(bg) + 0.0722 * luminance(bb);
  const lighter = Math.max(foregroundLuminance, backgroundLuminance);
  const darker = Math.min(foregroundLuminance, backgroundLuminance);
  return (lighter + 0.05) / (darker + 0.05);
}

async function assertGalleryColors(canvasElement: HTMLElement) {
  const chips = Array.from(
    canvasElement.querySelectorAll<HTMLElement>(
      "[data-testid^='chip-']:not([data-testid='chip-gallery'])",
    ),
  );
  expect(chips).toHaveLength(18);
  for (const chip of chips) {
    const styles = getComputedStyle(chip);
    const ratio = contrastRatio(styles.color, styles.backgroundColor);
    expect(ratio, `${chip.dataset.testid} contrast`).toBeGreaterThanOrEqual(4.5);
    expect(styles.color).not.toMatch(/(?:rgb\(0\s*,\s*0\s*,\s*0\)|rgb\(255\s*,\s*255\s*,\s*255\))/);
  }

  const focusChip = canvasElement.querySelector<HTMLElement>(
    "[data-testid='focus-indicator-chip']",
  );
  expect(focusChip).not.toBeNull();
  focusChip!.focus({ focusVisible: true } as FocusOptions);
  expect(document.activeElement).toBe(focusChip);
  expect(focusChip!.matches(":focus-visible")).toBe(true);
  const focusStyles = getComputedStyle(focusChip!);
  expect(focusChip).toHaveClass("focus-visible:outline-2", "focus-visible:outline-primary");
  expect(focusStyles.outlineWidth).toBe("2px");
  const focusColor = focusStyles.outlineColor;
  expect(contrastRatio(focusColor, focusStyles.backgroundColor)).toBeGreaterThanOrEqual(3);
}

const meta = {
  title: "UI/Chip",
  component: Chip,
  tags: ["autodocs", "test"],
  decorators: [
    (Story) => (
      <StorySurface>
        <Story />
      </StorySurface>
    ),
  ],
} satisfies Meta<typeof Chip>;

export default meta;

type Story = StoryObj<typeof meta>;

export const GalleryLight: Story = {
  render: () => <ChipGallery />,
  globals: { themeMode: "light" },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText("18 explicit tone presets")).toBeVisible();
    await assertGalleryColors(canvasElement);
  },
};

export const GalleryDark: Story = {
  render: () => <ChipGallery />,
  globals: { themeMode: "dark" },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText("18 explicit tone presets")).toBeVisible();
    await assertGalleryColors(canvasElement);
  },
};
