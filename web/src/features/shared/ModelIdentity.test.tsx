import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import {
  ModelIdentity,
  resolveModelIdentityGeneration,
  resolveModelIdentityGenerationLabel,
  resolveModelIdentityIcon,
} from "./ModelIdentity";

describe("resolveModelIdentityIcon", () => {
  it.each([
    ["gpt-5.6-sol", "white-balance-sunny"],
    ["gpt-5.6", "white-balance-sunny"],
    ["gpt-5.6-terra", "earth"],
    ["gpt-5.6-luna", "weather-night"],
    ["gpt-5.6-sol-2026-07-08", "white-balance-sunny"],
    ["gpt-5.6-terra-2026-07-08", "earth"],
    ["gpt-5.6-luna-2026-07-08", "weather-night"],
  ])("maps %s to %s", (model, iconName) => {
    expect(resolveModelIdentityIcon(model)).toBe(iconName);
  });

  it.each([
    ["gpt-6-astra", "creation"],
    ["gpt-6-sol", "white-balance-sunny"],
    ["gpt-6-luna", "weather-night"],
    ["gpt-6.1-astra", "creation"],
    ["gpt-6.1-sol", "white-balance-sunny"],
    ["gpt-6.1-luna", "weather-night"],
    ["gpt-6-astra-2024-02-29", "creation"],
    ["GPT-6.1-SOL-2026-09-25", "white-balance-sunny"],
    ["  GPT-6.1-LUNA  ", "weather-night"],
  ])("maps %s to the family icon %s", (model, iconName) => {
    expect(resolveModelIdentityIcon(model)).toBe(iconName);
    expect(resolveModelIdentityGeneration(model)).toBe(6);
  });

  it.each([
    ["gpt-5.6", 5],
    ["gpt-5.6-sol", 5],
    ["gpt-5.6-terra", 5],
    ["gpt-5.6-luna", 5],
    ["gpt-6-astra", 6],
    ["gpt-6-sol", 6],
    ["gpt-6-luna", 6],
    ["gpt-6.1-astra", 6],
    ["gpt-6.1-sol", 6],
    ["gpt-6.1-luna", 6],
  ] as const)("resolves the major generation for %s", (model, generation) => {
    expect(resolveModelIdentityGeneration(model)).toBe(generation);
  });

  it.each([
    ["gpt-5.6", "5.6"],
    ["gpt-5.6-sol-2026-02-28", "5.6"],
    ["GPT-6-SOL-2024-02-29", "6"],
    ["gpt-6.1-astra-2026-01-01", "6.1"],
    [" gpt-6.1-luna ", "6.1"],
  ] as const)("resolves full generation label for %s", (model, generation) => {
    expect(resolveModelIdentityGenerationLabel(model)).toBe(generation);
  });

  it.each([
    "gpt-5.5",
    "gpt-5.6-sol-preview",
    "gpt-5.6-2026-01-01",
    "gpt-5.6-sol-2026-02-30",
    "gpt-6",
    "gpt-6.1",
    "gpt-6-terra",
    "gpt-6.1-terra",
    "gpt-6-astra-preview",
    "gpt-6-astra-2025-02-29",
    "gpt-6-sol-2026-02-30",
    "gpt-6.1-sol-2025-02-29",
    "custom-model",
    "",
  ])("does not map %s", (model) => {
    expect(resolveModelIdentityIcon(model)).toBeNull();
    expect(resolveModelIdentityGenerationLabel(model)).toBeNull();
  });

  it.each([
    "gpt-6",
    "gpt-6-terra",
    "gpt-6-astra-preview",
    "gpt-6-astra-2025-02-29",
  ])("does not resolve an unsupported GPT-6 id %s", (model) => {
    expect(resolveModelIdentityGeneration(model)).toBeNull();
  });
});

describe("ModelIdentity", () => {
  it.each([
    ["gpt-5.6-sol", "white-balance-sunny", "text-warning"],
    ["gpt-5.6-terra", "earth", "text-success"],
    ["gpt-5.6-luna", "weather-night", "text-info"],
    ["gpt-6-sol", "white-balance-sunny", "text-warning"],
    ["gpt-6-luna", "weather-night", "text-info"],
    ["gpt-6.1-sol", "white-balance-sunny", "text-warning"],
    ["gpt-6.1-luna", "weather-night", "text-info"],
  ])("renders %s with its fixed identity color", (model, iconName, colorClassName) => {
    const markup = renderToStaticMarkup(<ModelIdentity model={model} />);

    expect(markup).toContain(`data-model-icon="${iconName}"`);
    expect(markup).toContain(colorClassName);
  });

  it("renders a target model as an icon with the complete accessible model id", () => {
    const markup = renderToStaticMarkup(
      <ModelIdentity model="gpt-5.6-terra-2026-07-08" testId="model-identity" />,
    );

    expect(markup).toContain('data-model-icon="earth"');
    expect(markup).toContain('aria-label="gpt-5.6-terra-2026-07-08"');
    expect(markup).toContain('title="gpt-5.6-terra-2026-07-08"');
    expect(markup).toContain("text-success");
    expect(markup).not.toContain(">gpt-5.6-terra-2026-07-08<");
  });

  it("keeps non-target models as text", () => {
    const markup = renderToStaticMarkup(<ModelIdentity model="gpt-5.5" />);

    expect(markup).toContain(">gpt-5.5<");
    expect(markup).not.toContain("data-model-icon");
  });

  it("uses the Sol identity for the gpt-5.6 alias", () => {
    const markup = renderToStaticMarkup(<ModelIdentity model="gpt-5.6" />);

    expect(markup).toContain('data-model-icon="white-balance-sunny"');
    expect(markup).toContain("text-warning");
  });

  it("exposes the GPT-6 identity without a generation-specific presentation mode", () => {
    const markup = renderToStaticMarkup(<ModelIdentity model="gpt-6-astra" />);

    expect(markup).toContain('data-model-generation="6"');
    expect(markup).toContain('data-model-generation-label="6"');
    expect(markup).toContain('data-model-variant="astra"');
    expect(markup).toContain('data-model-icon="creation"');
    expect(markup).toContain("model-identity-astra");
    expect(markup).not.toContain("data-model-image");
    expect(markup).not.toContain("data-model-presentation");
    expect(markup).not.toContain("model-identity-gpt6");
    expect(markup).toContain('aria-label="gpt-6-astra"');
  });

  it("keeps an invalid GPT-6 dated id visible as text", () => {
    const markup = renderToStaticMarkup(<ModelIdentity model="gpt-6-astra-2026-02-30" />);

    expect(markup).toContain(">gpt-6-astra-2026-02-30<");
    expect(markup).not.toContain("data-model-icon");
  });

  it("exposes the full GPT-6.1 generation label while keeping GPT-6 styling", () => {
    const markup = renderToStaticMarkup(<ModelIdentity model="gpt-6.1-luna" />);

    expect(markup).toContain('data-model-generation="6"');
    expect(markup).toContain('data-model-generation-label="6.1"');
    expect(markup).toContain('data-model-icon="weather-night"');
    expect(markup).not.toContain("data-model-presentation");
  });
});
