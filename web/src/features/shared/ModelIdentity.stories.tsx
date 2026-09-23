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
    viewport: { defaultViewport: "desktop1660" },
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
    await expect(canvas.getByTestId(`model-${model}`)).toHaveAttribute("aria-label", model);
    await expect(canvas.getByText(label)).toBeVisible();
  }

  const gpt6Models = [
    ["gpt-6-astra", "creation"],
    ["gpt-6-sol", "weather-sunny"],
    ["gpt-6-luna", "moon-waning-crescent"],
  ] as const;
  const colorMode = canvasElement.ownerDocument.documentElement.getAttribute("data-color-mode");
  const expectedColors =
    colorMode === "dark"
      ? ["rgb(187, 166, 246)", "rgb(255, 177, 106)", "rgb(100, 209, 199)"]
      : ["rgb(103, 73, 186)", "rgb(169, 80, 24)", "rgb(4, 118, 111)"];
  for (const [model, iconName] of gpt6Models) {
    const identity = canvas.getByTestId(`model-${model}`);
    await expect(identity).toHaveAttribute("data-model-icon", iconName);
    await expect(identity.querySelector("svg")).not.toBeNull();
    await expect(identity).toHaveClass("h-5", "w-5");
    expect(getComputedStyle(identity).color).toBe(
      expectedColors[gpt6Models.findIndex(([id]) => id === model)],
    );
  }
}

export const GenerationComparisonLight: Story = {
  globals: {
    themeMode: "light",
    viewport: { value: "desktop1660", isRotated: false },
  },
  render: () => <IdentityGallery />,
  play: comparisonPlay,
};

export const GenerationComparisonDark: Story = {
  globals: {
    themeMode: "dark",
    viewport: { value: "desktop1660", isRotated: false },
  },
  render: () => <IdentityGallery />,
  play: comparisonPlay,
};

export const GenerationComparisonMobile393: Story = {
  globals: {
    themeMode: "light",
    viewport: { value: "mobile393", isRotated: false },
  },
  render: () => <IdentityGallery />,
  play: comparisonPlay,
};

function PresentationModesGallery() {
  return (
    <div className="grid max-w-2xl gap-5 text-sm sm:grid-cols-3">
      <div className="flex items-center gap-3">
        <ModelIdentity model="gpt-6-astra" testId="gpt6-standalone" />
        <span>Standalone</span>
      </div>
      <div className="flex items-center gap-3">
        <ModelPerformanceModelIdentity
          model="gpt-6-sol"
          effortValue="high"
          testId="gpt6-model-badge"
        />
        <span>Outlined badge</span>
      </div>
      <div className="flex items-center gap-3">
        <span className="inline-flex min-h-4 items-center gap-1.5" data-testid="gpt6-chart-legend">
          <span className="h-[3px] w-7 flex-none rounded-full bg-primary" aria-hidden="true" />
          <ModelIdentity
            model="gpt-6-luna"
            presentation="compact"
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
  const colorMode = canvasElement.ownerDocument.documentElement.getAttribute("data-color-mode");
  const expectedTile = colorMode === "dark" ? "rgb(40, 52, 63)" : "rgb(241, 244, 247)";
  const expectedBorder = colorMode === "dark" ? "rgb(86, 104, 121)" : "rgb(205, 215, 225)";
  const standaloneStyle = getComputedStyle(canvas.getByTestId("gpt6-standalone"));
  expect(standaloneStyle.backgroundColor).toBe(expectedTile);
  expect(standaloneStyle.borderTopWidth).toBe("1px");
  expect(standaloneStyle.borderTopColor).toBe(expectedBorder);
  await expect(canvas.getByTestId("gpt6-standalone")).toHaveAttribute(
    "data-model-presentation",
    "standalone",
  );
  const embeddedIdentity = canvas
    .getByTestId("gpt6-model-badge")
    .querySelector<HTMLElement>('[data-model-presentation="embedded"]');
  await expect(embeddedIdentity).not.toBeNull();
  await expect(canvas.getByTestId("gpt6-legend-label")).toHaveTextContent("medium");
  await expect(canvas.getByTestId("gpt6-legend-identity")).toHaveAttribute(
    "data-model-presentation",
    "compact",
  );
  await expect(canvas.getByTestId("gpt6-legend-identity")).toHaveAttribute(
    "data-model-variant",
    "luna",
  );
  await expect(embeddedIdentity).toHaveClass("h-6", "w-6");
  expect(getComputedStyle(embeddedIdentity!).borderTopWidth).toBe("0px");
  expect(getComputedStyle(embeddedIdentity!).backgroundColor).toBe(expectedTile);

  const compactStyle = getComputedStyle(canvas.getByTestId("gpt6-legend-identity"));
  expect(compactStyle.borderTopWidth).toBe("0px");
  expect(compactStyle.backgroundColor).toBe("rgba(0, 0, 0, 0)");
}

export const PresentationModes: Story = {
  globals: {
    themeMode: "light",
    viewport: { value: "desktop1660", isRotated: false },
  },
  render: () => <PresentationModesGallery />,
  play: presentationModesPlay,
};

export const PresentationModesDark: Story = {
  globals: {
    themeMode: "dark",
    viewport: { value: "desktop1660", isRotated: false },
  },
  render: () => <PresentationModesGallery />,
  play: presentationModesPlay,
};

export const PresentationModesMobile393: Story = {
  globals: {
    themeMode: "light",
    viewport: { value: "mobile393", isRotated: false },
  },
  render: () => <PresentationModesGallery />,
  play: presentationModesPlay,
};

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
    await expect(canvas.getByTestId("model-sol").querySelector("svg")).toHaveClass("text-warning");
    await expect(canvas.getByTestId("model-terra").querySelector("svg")).toHaveClass(
      "text-success",
    );
    await expect(canvas.getByTestId("model-luna").querySelector("svg")).toHaveClass("text-info");
  },
};

export const GPT6AstraSolLuna: Story = {
  tags: ["test"],
  render: () => (
    <>
      <ModelIdentity model="gpt-6-astra" testId="model-astra" />
      <ModelIdentity model="gpt-6-sol" testId="model-gpt6-sol" />
      <ModelIdentity model="gpt-6-luna" testId="model-gpt6-luna" />
    </>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByTestId("model-astra")).toHaveAttribute(
      "data-model-icon",
      "star-four-points",
    );
    await expect(canvas.getByTestId("model-gpt6-sol")).toHaveAttribute(
      "data-model-icon",
      "white-balance-sunny",
    );
    await expect(canvas.getByTestId("model-gpt6-luna")).toHaveAttribute(
      "data-model-icon",
      "weather-night",
    );
  },
};

export const DatedVariantAndFallback: Story = {
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
