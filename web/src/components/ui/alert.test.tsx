import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { Alert } from "./alert";

describe("Alert variants", () => {
  it.each([
    ["info", "bg-info/10", "tone-ink-info", "text-info"],
    ["success", "bg-success/10", "tone-ink-success", "text-success"],
    ["warning", "bg-warning/15", "tone-ink-warning", "text-warning"],
    ["error", "bg-error/15", "tone-ink-error", "text-error"],
  ] as const)("keeps %s text on the shared low-opacity semantic ink contract", (variant, surfaceClass, inkClass, rawSemanticTextClass) => {
    const html = renderToStaticMarkup(<Alert variant={variant}>{variant}</Alert>);

    expect(html).toContain(surfaceClass);
    expect(html).toContain(inkClass);
    expect(html).not.toContain(`${rawSemanticTextClass}"`);
  });
});
