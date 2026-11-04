/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "../../i18n";
import type { ModelRoutingState } from "../../lib/api";
import { ModelRoutingHealthPanel } from "./ModelRoutingHealthPanel";

const states: ModelRoutingState[] = [
  {
    model: "blocked-model",
    state: "cooling_down",
    lastFailureKind: "model_quota",
    priority: "excluded",
    failureCount: 3,
    lastSeenAt: "2026-11-04T00:00:00Z",
  },
  {
    model: "healthy-model",
    state: "available",
    priority: "normal",
    failureCount: 0,
    lastSeenAt: "2026-11-04T00:00:00Z",
  },
];
const host = document.createElement("div");
let root = createRoot(host);
afterEach(() => {
  act(() => root.unmount());
  root = createRoot(host);
});
describe("model routing controls", () => {
  it("resets only the selected model and disables further writes while resetting", async () => {
    const onReset = vi.fn();
    await act(async () =>
      root.render(
        <I18nProvider>
          <ModelRoutingHealthPanel states={states} writesEnabled onReset={onReset} />
        </I18nProvider>,
      ),
    );
    const reset = host.querySelector<HTMLButtonElement>(
      '[data-testid="model-routing-reset-blocked-model"]',
    )!;
    act(() => reset.click());
    expect(onReset).toHaveBeenCalledExactlyOnceWith("blocked-model");
    expect(host.querySelector('[data-testid="model-routing-reset-healthy-model"]')).toBeNull();
    expect(host.textContent).toContain("Reserve 仅提示账号级额度");
    await act(async () =>
      root.render(
        <I18nProvider>
          <ModelRoutingHealthPanel
            states={states}
            writesEnabled
            resettingModel="blocked-model"
            onReset={onReset}
          />
        </I18nProvider>,
      ),
    );
    expect(
      host.querySelector<HTMLButtonElement>('[data-testid="model-routing-reset-blocked-model"]')!
        .disabled,
    ).toBe(true);
  });
  it("filters by model name without mutating route states", async () => {
    await act(async () =>
      root.render(
        <I18nProvider>
          <ModelRoutingHealthPanel states={states} writesEnabled onReset={vi.fn()} />
        </I18nProvider>,
      ),
    );
    const input = host.querySelector<HTMLInputElement>('input[aria-label="筛选模型"]')!;
    act(() => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(
        input,
        "healthy",
      );
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(host.textContent).toContain("healthy-model");
    expect(host.textContent).not.toContain("blocked-model");
  });
});
