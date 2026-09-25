import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { ModelPerformanceModelIdentity } from "./ModelPerformanceModelIdentity";

describe("ModelPerformanceModelIdentity", () => {
  it("uses the icon instead of a duplicate model name while preserving the hover label", () => {
    const markup = renderToStaticMarkup(
      <ModelPerformanceModelIdentity model="gpt-5.6" effortValue=" MAX " testId="model-context" />,
    );

    expect(markup).toContain('data-model-context-display="model-badge"');
    expect(markup).toContain('title="gpt-5.6 · max"');
    expect(markup).toContain('data-model-icon="white-balance-sunny"');
    expect(markup).toContain('data-reasoning-effort-tone="max"');
    expect(markup).not.toContain('data-testid="model-context-name"');
    expect(markup).not.toContain(">gpt-5.6<");
    const identity = markup.match(/<span(?=[^>]*data-model-icon="white-balance-sunny")[^>]*>/)?.[0];
    expect(identity).toContain("h-5 w-5");
    expect(identity).not.toContain("h-6 w-6");
  });

  it("keeps an unspecified effort as the shared fallback", () => {
    const markup = renderToStaticMarkup(
      <ModelPerformanceModelIdentity
        model="gpt-5.6-luna"
        effortValue={null}
        testId="model-context"
      />,
    );

    expect(markup).toContain("gpt-5.6-luna · —");
    expect(markup).toContain('data-reasoning-effort-tone="none"');
    expect(markup).toContain('data-testid="model-context-effort-marker"');
    expect(markup).toContain("bg-base-content/45");
  });

  it("integrates a GPT-6 identity into the existing model badge segment", () => {
    const markup = renderToStaticMarkup(
      <ModelPerformanceModelIdentity
        model="gpt-6-sol-2026-09-25"
        effortValue="high"
        testId="model-context"
      />,
    );

    expect(markup).toContain('data-model-context-display="model-badge"');
    expect(markup).toContain('data-model-presentation="embedded"');
    expect(markup).toContain('data-model-icon="weather-sunny"');
    expect(markup).toContain('aria-label="gpt-6-sol-2026-09-25"');
    expect(markup).toContain("h-6 w-6");
    const embeddedIdentity = markup.match(
      /<span(?=[^>]*data-model-presentation="embedded")[^>]*>/,
    )?.[0];
    expect(embeddedIdentity).toBeDefined();
    expect(embeddedIdentity).not.toContain("border");
    expect(markup.match(/class="[^"]*\bborder\b[^"]*"/g)).toHaveLength(1);
    expect(markup).not.toContain('data-testid="model-context-name"');
  });
});
