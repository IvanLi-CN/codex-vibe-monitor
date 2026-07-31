import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { ModelPerformanceModelIdentity } from "./ModelPerformanceModelIdentity";

describe("ModelPerformanceModelIdentity", () => {
  it("uses the icon instead of a duplicate model name while preserving the hover label", () => {
    const markup = renderToStaticMarkup(
      <ModelPerformanceModelIdentity
        model="gpt-5.6-sol"
        effort="high"
        effortValue="high"
        testId="model-context"
      />,
    );

    expect(markup).toContain('data-model-context-display="model-badge"');
    expect(markup).toContain('title="gpt-5.6-sol · high"');
    expect(markup).toContain('data-model-icon="white-balance-sunny"');
    expect(markup).not.toContain('data-testid="model-context-name"');
    expect(markup).not.toContain(">gpt-5.6-sol<");
  });
});
