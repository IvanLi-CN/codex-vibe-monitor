import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { ModelIdentity, resolveModelIdentityIcon } from "./ModelIdentity";

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
