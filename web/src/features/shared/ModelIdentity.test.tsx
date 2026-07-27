import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import {
  isCompleteGpt56ModelSet,
  ModelIdentity,
  ModelIdentityGroup,
  resolveModelIdentityIcon,
} from "./ModelIdentity";

describe("resolveModelIdentityIcon", () => {
  it.each([
    ["gpt-5.6-sol", "white-balance-sunny"],
    ["gpt-5.6-terra", "earth"],
    ["gpt-5.6-luna", "weather-night"],
    ["gpt-5.6-sol-2026-07-08", "white-balance-sunny"],
    ["gpt-5.6-terra-2026-07-08", "earth"],
    ["gpt-5.6-luna-2026-07-08", "weather-night"],
  ])("maps %s to %s", (model, iconName) => {
    expect(resolveModelIdentityIcon(model)).toBe(iconName);
  });

  it.each([
    "gpt-5.6",
    "gpt-5.5",
    "gpt-5.6-sol-preview",
    "custom-model",
    "",
  ])("does not map %s", (model) => {
    expect(resolveModelIdentityIcon(model)).toBeNull();
  });
});

describe("ModelIdentity", () => {
  it("renders a target model as an icon with the complete accessible model id", () => {
    const markup = renderToStaticMarkup(
      <ModelIdentity model="gpt-5.6-terra-2026-07-08" testId="model-identity" />,
    );

    expect(markup).toContain('data-model-icon="earth"');
    expect(markup).toContain('aria-label="gpt-5.6-terra-2026-07-08"');
    expect(markup).toContain('title="gpt-5.6-terra-2026-07-08"');
    expect(markup).not.toContain(">gpt-5.6-terra-2026-07-08<");
  });

  it("keeps non-target models as text", () => {
    const markup = renderToStaticMarkup(<ModelIdentity model="gpt-5.5" />);

    expect(markup).toContain(">gpt-5.5<");
    expect(markup).not.toContain("data-model-icon");
  });
});

describe("ModelIdentityGroup", () => {
  it.each([
    ["canonical ids", ["gpt-5.6-luna", "gpt-5.6-sol", "gpt-5.6-terra"]],
    [
      "dated ids",
      ["gpt-5.6-luna-2026-07-08", "gpt-5.6-sol-2026-07-08", "gpt-5.6-terra-2026-07-08"],
    ],
  ])("groups the complete %s family", (_label, models) => {
    expect(isCompleteGpt56ModelSet(models)).toBe(true);
    const markup = renderToStaticMarkup(
      <ModelIdentityGroup models={models} testId="model-group" />,
    );

    expect(markup).toContain('data-model-identity-group="gpt-5.6"');
    expect(markup).toContain('aria-label="gpt-5.6-sol');
    expect(markup).toContain('data-model-icon="white-balance-sunny"');
    expect(markup).toContain('data-model-icon="earth"');
    expect(markup).toContain('data-model-icon="weather-night"');
  });

  it("does not group an incomplete or non-target set", () => {
    expect(isCompleteGpt56ModelSet(["gpt-5.6-sol", "gpt-5.6-terra"])).toBe(false);
    expect(renderToStaticMarkup(<ModelIdentityGroup models={["gpt-5.6-sol", "gpt-5.5"]} />)).toBe(
      "",
    );
  });
});
