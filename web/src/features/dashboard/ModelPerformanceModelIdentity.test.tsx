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
    expect(markup).not.toContain('data-testid="model-context-effort-marker"');
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
    expect(markup).not.toContain('data-testid="model-context-effort-marker"');
    expect(markup).not.toContain('data-testid="model-context-generation"');
    expect(markup).toContain("text-base-content/68");
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
    expect(markup).toContain('data-model-icon="white-balance-sunny"');
    expect(markup).toContain('aria-label="gpt-6-sol-2026-09-25"');
    expect(markup).toContain("h-5 w-5");
    const embeddedIdentity = markup.match(
      /<span(?=[^>]*data-model-icon="white-balance-sunny")[^>]*>/,
    )?.[0];
    expect(embeddedIdentity).toBeDefined();
    expect(embeddedIdentity).not.toContain("border");
    expect(markup).not.toContain("data-model-presentation");
    expect(markup.match(/class="[^"]*\bborder\b[^"]*"/g)).toHaveLength(1);
    expect(markup).not.toContain('data-testid="model-context-name"');
  });

  it("opts into generation before the fixed family icon and reasoning effort", () => {
    const markup = renderToStaticMarkup(
      <ModelPerformanceModelIdentity
        model="gpt-6.1-luna-2026-09-25"
        effortValue="high"
        showGeneration
        testId="model-context"
      />,
    );

    const generationIndex = markup.indexOf('data-testid="model-context-generation"');
    const iconIndex = markup.indexOf('data-model-icon="weather-night"');
    const effortIndex = markup.indexOf('data-testid="model-context-effort"');
    expect(generationIndex).toBeGreaterThan(-1);
    expect(iconIndex).toBeGreaterThan(generationIndex);
    expect(effortIndex).toBeGreaterThan(iconIndex);
    expect(markup).toContain(">6.1</span>");
    expect(markup).toContain('title="gpt-6.1-luna-2026-09-25 · high"');
    expect(markup).toContain('data-testid="model-context-badge"');
    expect(markup).toContain("h-5 w-5");
    expect(markup).not.toContain(">gpt-6.1-luna-2026-09-25<");
  });

  it("keeps unspecified and explicit none reasoning effort distinct", () => {
    const missingValues = [null, undefined, "", "  ", " — "] as const;
    for (const [index, effortValue] of missingValues.entries()) {
      const markup = renderToStaticMarkup(
        <ModelPerformanceModelIdentity
          model="gpt-5.6-sol"
          effortValue={effortValue}
          showGeneration
          testId={`missing-${index}`}
        />,
      );
      expect(markup).toContain(`title="gpt-5.6-sol · —"`);
      expect(markup).toContain('data-reasoning-effort-tone="none"');
      expect(markup).toContain(">—</span>");
    }
    const noneMarkup = renderToStaticMarkup(
      <ModelPerformanceModelIdentity
        model="gpt-5.6-sol"
        effortValue="none"
        showGeneration
        testId="none"
      />,
    );

    expect(noneMarkup).toContain('title="gpt-5.6-sol · none"');
    expect(noneMarkup).toContain(">none</span>");
  });

  it("keeps unknown effort text in the existing fallback tone", () => {
    const markup = renderToStaticMarkup(
      <ModelPerformanceModelIdentity
        model="gpt-6-sol"
        effortValue=" Experimental "
        showGeneration
        testId="custom"
      />,
    );

    expect(markup).toContain(">experimental</span>");
    expect(markup).toContain('data-reasoning-effort-tone="unknown"');
    expect(markup).toContain('title="gpt-6-sol · experimental"');
  });
});
