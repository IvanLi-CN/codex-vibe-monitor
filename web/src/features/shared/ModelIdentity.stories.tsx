import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, within } from "storybook/test";
import { ModelPerformanceModelIdentity } from "../dashboard/ModelPerformanceModelIdentity";
import { ModelIdentity } from "./ModelIdentity";

const modelSamples = [
  ["GPT-5.6 Sol", "gpt-5.6-sol"],
  ["GPT-5.6 Terra", "gpt-5.6-terra"],
  ["GPT-5.6 Luna", "gpt-5.6-luna"],
  ["GPT-6 Astra", "gpt-6-astra"],
  ["GPT-6 Sol", "gpt-6-sol"],
  ["GPT-6 Luna", "gpt-6-luna"],
  ["GPT-6.1 Astra", "gpt-6.1-astra"],
  ["GPT-6.1 Sol", "gpt-6.1-sol"],
  ["GPT-6.1 Luna", "gpt-6.1-luna"],
] as const;

function IdentityGallery() {
  return (
    <div className="grid max-w-2xl grid-cols-2 gap-x-8 gap-y-5 text-sm sm:grid-cols-3">
      {modelSamples.map(([label, model]) => (
        <div key={model} className="flex items-center gap-3">
          <ModelIdentity model={model} testId={`model-${model}`} />
          <span>{label}</span>
        </div>
      ))}
    </div>
  );
}

const meta = {
  title: "Components/ModelIdentity",
  component: ModelIdentity,
  tags: ["autodocs"],
  parameters: {
    layout: "fullscreen",
    viewport: { defaultViewport: "desktop1280" },
  },
  decorators: [
    (Story) => (
      <div
        data-visual-evidence-surface="model-identity-story-surface"
        className="min-h-screen bg-base-100 p-6 text-base-content"
      >
        <div data-visual-evidence-target="model-identity-story-target" className="inline-block">
          <Story />
        </div>
      </div>
    ),
  ],
  args: { model: "gpt-5.6-sol" },
} satisfies Meta<typeof ModelIdentity>;

export default meta;
type Story = StoryObj<typeof meta>;

async function comparisonPlay({ canvasElement }: { canvasElement: HTMLElement }) {
  const canvas = within(canvasElement);
  for (const [label, model] of modelSamples) {
    const identity = canvas.getByTestId(`model-${model}`);
    await expect(identity).toHaveAttribute("aria-label", model);
    await expect(canvas.getByText(label)).toBeVisible();
    const style = getComputedStyle(identity);
    expect(style.backgroundColor).toBe("rgba(0, 0, 0, 0)");
    expect(style.borderTopWidth).toBe("0px");
    expect(style.borderRightWidth).toBe("0px");
    expect(style.borderBottomWidth).toBe("0px");
    expect(style.borderLeftWidth).toBe("0px");
    expect(style.outlineStyle).toBe("none");
    expect(style.boxShadow).toBe("none");
    expect(style.borderTopLeftRadius).toBe("0px");
  }

  const gpt6Models = [
    ["gpt-6-astra", "creation", "astra"],
    ["gpt-6-sol", "white-balance-sunny", "sol"],
    ["gpt-6-luna", "weather-night", "luna"],
    ["gpt-6.1-astra", "creation", "astra"],
    ["gpt-6.1-sol", "white-balance-sunny", "sol"],
    ["gpt-6.1-luna", "weather-night", "luna"],
  ] as const;
  const colorMode = canvasElement.ownerDocument.documentElement.getAttribute("data-color-mode");
  const expectedColors = {
    astra: colorMode === "dark" ? "rgb(187, 166, 246)" : "rgb(103, 73, 186)",
    sol: getComputedStyle(canvas.getByTestId("model-gpt-5.6-sol")).color,
    luna: getComputedStyle(canvas.getByTestId("model-gpt-5.6-luna")).color,
  };
  for (const [model, iconName, family] of gpt6Models) {
    const identity = canvas.getByTestId(`model-${model}`);
    await expect(identity).toHaveAttribute("data-model-icon", iconName);
    await expect(identity).toHaveClass("h-5", "w-5");
    expect(getComputedStyle(identity).color).toBe(expectedColors[family]);
    await expect(identity).toHaveAttribute(
      "data-model-generation-label",
      model.startsWith("gpt-6.1") ? "6.1" : "6",
    );
  }
}

export const GenerationComparisonLight: Story = {
  tags: ["test"],
  globals: {
    themeMode: "light",
    viewport: { value: "desktop1280", isRotated: false },
  },
  render: () => <IdentityGallery />,
  play: comparisonPlay,
};

export const GenerationComparisonDark: Story = {
  tags: ["test"],
  globals: {
    themeMode: "dark",
    viewport: { value: "desktop1280", isRotated: false },
  },
  render: () => <IdentityGallery />,
  play: comparisonPlay,
};

export const GenerationComparisonMobile393: Story = {
  tags: ["test"],
  globals: {
    themeMode: "light",
    viewport: { value: "mobile393", isRotated: false },
  },
  render: () => <IdentityGallery />,
  play: comparisonPlay,
};

export const GenerationComparisonTablet: Story = {
  tags: ["test"],
  globals: {
    themeMode: "light",
    viewport: { value: "tablet768", isRotated: false },
  },
  render: () => <IdentityGallery />,
  play: comparisonPlay,
};

function PresentationModesGallery() {
  return (
    <div className="grid max-w-2xl gap-5 text-sm sm:grid-cols-3">
      <div className="flex items-center gap-3">
        <ModelIdentity model="gpt-6-astra" testId="gpt6-standalone" />
        <span>Standalone glyph</span>
      </div>
      <div className="flex items-center gap-3">
        <ModelPerformanceModelIdentity
          model="gpt-6.1-sol"
          effortValue="high"
          showGeneration
          testId="gpt6-model-badge"
        />
        <span>Detailed capsule</span>
      </div>
      <div className="flex items-center gap-3">
        <span className="inline-flex min-h-4 items-center gap-1.5" data-testid="gpt6-chart-legend">
          <span className="h-[3px] w-7 flex-none rounded-full bg-primary" aria-hidden="true" />
          <ModelIdentity
            model="gpt-6-luna"
            className="h-4 w-4"
            iconClassName="h-4 w-4"
            testId="gpt6-legend-identity"
          />
          <span data-testid="gpt6-legend-label">medium</span>
        </span>
        <span>Chart legend</span>
      </div>
    </div>
  );
}

async function presentationModesPlay({ canvasElement }: { canvasElement: HTMLElement }) {
  const canvas = within(canvasElement);
  const standalone = canvas.getByTestId("gpt6-standalone");
  const embeddedIdentity = canvas
    .getByTestId("gpt6-model-badge")
    .querySelector<HTMLElement>('[data-model-icon="white-balance-sunny"]');
  await expect(embeddedIdentity).not.toBeNull();
  await expect(canvas.getByTestId("gpt6-legend-label")).toHaveTextContent("medium");
  await expect(canvas.getByTestId("gpt6-legend-identity")).toHaveAttribute(
    "data-model-variant",
    "luna",
  );
  await expect(standalone).toHaveClass("model-identity-astra");
  await expect(embeddedIdentity).toHaveClass("h-5", "w-5");
  await expect(canvas.getByTestId("gpt6-model-badge-generation")).toHaveTextContent("6.1");
  const badge = canvas.getByTestId("gpt6-model-badge-badge");
  await expect(badge).toHaveClass("h-6", "border");
  for (const identity of [
    standalone,
    embeddedIdentity!,
    canvas.getByTestId("gpt6-legend-identity"),
  ]) {
    const style = getComputedStyle(identity);
    expect(style.backgroundColor).toBe("rgba(0, 0, 0, 0)");
    expect(style.borderTopWidth).toBe("0px");
    expect(style.outlineStyle).toBe("none");
    expect(style.boxShadow).toBe("none");
    expect(style.borderTopLeftRadius).toBe("0px");
  }
}

export const PresentationModes: Story = {
  tags: ["test"],
  globals: {
    themeMode: "light",
    viewport: { value: "desktop1660", isRotated: false },
  },
  render: () => <PresentationModesGallery />,
  play: presentationModesPlay,
};

export const PresentationModesDark: Story = {
  tags: ["test"],
  globals: {
    themeMode: "dark",
    viewport: { value: "desktop1660", isRotated: false },
  },
  render: () => <PresentationModesGallery />,
  play: presentationModesPlay,
};

export const PresentationModesMobile393: Story = {
  tags: ["test"],
  globals: {
    themeMode: "light",
    viewport: { value: "mobile393", isRotated: false },
  },
  render: () => <PresentationModesGallery />,
  play: presentationModesPlay,
};

export const PresentationModesTablet: Story = {
  tags: ["test"],
  globals: {
    themeMode: "light",
    viewport: { value: "tablet768", isRotated: false },
  },
  render: () => <PresentationModesGallery />,
  play: presentationModesPlay,
};

export const SolTerraLuna: Story = {
  tags: ["test"],
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
    await expect(canvas.getByTestId("model-sol")).toHaveAttribute(
      "data-model-icon",
      "white-balance-sunny",
    );
    await expect(canvas.getByTestId("model-terra")).toHaveAttribute("data-model-icon", "earth");
    await expect(canvas.getByTestId("model-luna")).toHaveAttribute(
      "data-model-icon",
      "weather-night",
    );
  },
};

export const GPT6AstraSolLuna: Story = {
  tags: ["test"],
  render: () => (
    <>
      <ModelIdentity model="gpt-6-astra" testId="model-astra" />
      <ModelIdentity model="gpt-6-sol" testId="model-gpt6-sol" />
      <ModelIdentity model="gpt-6-luna" testId="model-gpt6-luna" />
      <ModelIdentity model="gpt-6.1-astra" testId="model-gpt61-astra" />
      <ModelIdentity model="gpt-6.1-sol" testId="model-gpt61-sol" />
      <ModelIdentity model="gpt-6.1-luna" testId="model-gpt61-luna" />
    </>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByTestId("model-astra")).toHaveAttribute("data-model-icon", "creation");
    await expect(canvas.getByTestId("model-gpt6-sol")).toHaveAttribute(
      "data-model-icon",
      "white-balance-sunny",
    );
    await expect(canvas.getByTestId("model-gpt6-luna")).toHaveAttribute(
      "data-model-icon",
      "weather-night",
    );
    await expect(canvas.getByTestId("model-gpt61-astra")).toHaveAttribute(
      "data-model-generation-label",
      "6.1",
    );
    await expect(canvas.getByTestId("model-gpt61-sol")).toHaveAttribute(
      "data-model-icon",
      "white-balance-sunny",
    );
    await expect(canvas.getByTestId("model-gpt61-luna")).toHaveAttribute(
      "data-model-icon",
      "weather-night",
    );
  },
};

export const DatedVariantAndFallback: Story = {
  tags: ["test"],
  render: () => (
    <>
      <ModelIdentity model="gpt-5.6" testId="model-alias" />
      <ModelIdentity model="gpt-5.6-sol-2026-07-08" testId="model-dated" />
      <ModelIdentity model="gpt-5.5" testId="model-fallback" />
    </>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByTestId("model-alias")).toHaveAttribute(
      "data-model-icon",
      "white-balance-sunny",
    );
    await expect(canvas.getByTestId("model-dated")).toHaveAttribute(
      "data-model-icon",
      "white-balance-sunny",
    );
    await expect(canvas.getByTestId("model-fallback")).toHaveTextContent("gpt-5.5");
  },
};
