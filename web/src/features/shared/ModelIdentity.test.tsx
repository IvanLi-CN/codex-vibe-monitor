import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import {
  ModelIdentity,
  resolveModelIdentityGeneration,
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
    ["gpt-6-sol", "weather-sunny"],
    ["gpt-6-luna", "moon-waning-crescent"],
    ["gpt-6-astra-2024-02-29", "creation"],
    ["GPT-6-SOL-2026-09-25", "weather-sunny"],
  ])("maps %s to the generic GPT-6 fallback icon %s", (model, iconName) => {
    expect(resolveModelIdentityIcon(model)).toBe(iconName);
    expect(resolveModelIdentityGeneration(model)).toBe(6);
  });

  it.each([
    ["gpt-5.6-sol", 5],
    ["gpt-5.6-terra", 5],
    ["gpt-5.6-luna", 5],
    ["gpt-6-astra", 6],
    ["gpt-6-sol", 6],
    ["gpt-6-luna", 6],
  ])("resolves the generation for %s", (model, generation) => {
    expect(resolveModelIdentityGeneration(model)).toBe(generation);
  });

  it.each([
    "gpt-5.5",
    "gpt-5.6-sol-preview",
    "gpt-6",
    "gpt-6-terra",
    "gpt-6-astra-preview",
    "gpt-6-astra-2025-02-29",
    "gpt-6-sol-2026-02-30",
    "custom-model",
    "",
  ])("does not map %s", (model) => {
    expect(resolveModelIdentityIcon(model)).toBeNull();
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

  it.each([
    "standalone",
    "embedded",
    "compact",
  ] as const)("exposes the %s GPT-6 presentation and complete accessible id", (presentation) => {
    const markup = renderToStaticMarkup(
      <ModelIdentity model="gpt-6-astra" presentation={presentation} />,
    );

    expect(markup).toContain(`data-model-presentation="${presentation}"`);
    expect(markup).toContain('data-model-generation="6"');
    expect(markup).toContain('data-model-variant="astra"');
    expect(markup).toContain('data-model-icon="creation"');
    expect(markup).not.toContain("data-model-image");
    expect(markup).toContain('aria-label="gpt-6-astra"');
  });

  it("keeps an invalid GPT-6 dated id visible as text", () => {
    const markup = renderToStaticMarkup(<ModelIdentity model="gpt-6-astra-2026-02-30" />);

    expect(markup).toContain(">gpt-6-astra-2026-02-30<");
    expect(markup).not.toContain("data-model-icon");
  });
});
