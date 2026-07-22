/** @vitest-environment jsdom */
import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { InvocationModelFilterField } from "./invocation-model-filter-field";

let host: HTMLDivElement | null = null;
let root: Root | null = null;

beforeAll(() => {
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
    configurable: true,
    writable: true,
    value: true,
  });
  if (typeof HTMLElement.prototype.scrollIntoView !== "function") {
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
      configurable: true,
      writable: true,
      value: () => undefined,
    });
  }
});

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
});

function render(ui: React.ReactNode) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(ui);
  });
}

function setNativeValue(element: HTMLInputElement, value: string) {
  const descriptor = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value");
  descriptor?.set?.call(element, value);
}

function ControlledField() {
  const [value, setValue] = useState({
    modelTarget: "request" as const,
    modelRerouted: "all" as const,
    models: ["gpt-5.4"],
    reasoningEfforts: [],
  });
  const [modelInputValue, setModelInputValue] = useState("");
  const [reasoningInputValue, setReasoningInputValue] = useState("");

  return (
    <InvocationModelFilterField
      testId="model-filter-field"
      label="Model"
      hint="Compound control"
      value={value}
      onChange={setValue}
      modelLabel="Model"
      reasoningEffortLabel="Reasoning effort"
      modelTargetLabel="Match side"
      requestTargetLabel="Request"
      responseTargetLabel="Response"
      reroutedLabel="Reroute"
      reroutedAllLabel="All"
      reroutedOnlyLabel="Rerouted"
      notReroutedLabel="Not rerouted"
      modelInputValue={modelInputValue}
      onModelInputValueChange={setModelInputValue}
      modelOptions={["gpt-5.4", "gpt-5", "gpt-5-mini"]}
      modelPlaceholder="Add model"
      modelInputId="model-filter-model-input"
      reasoningEffortInputValue={reasoningInputValue}
      onReasoningEffortInputValueChange={setReasoningInputValue}
      reasoningEffortOptions={["low", "medium", "high"]}
      reasoningEffortPlaceholder="Add reasoning effort"
      reasoningEffortInputId="model-filter-reasoning-effort-input"
      emptyText="No matches"
      loadingText="Searching…"
      addLabel="Add"
    />
  );
}

describe("InvocationModelFilterField", () => {
  it("keeps request/response, rerouted, model, and reasoning inside one compound field", () => {
    render(<ControlledField />);

    const field = document.querySelector('[data-testid="model-filter-field"]');
    if (!(field instanceof HTMLDivElement)) {
      throw new Error("missing model filter field");
    }

    const panelTrigger = field.querySelector('[data-testid="model-filter-field-trigger"]');

    expect(field.querySelector('[data-testid="model-filter-field-target-response"]')).toBeNull();

    if (!(panelTrigger instanceof HTMLButtonElement)) {
      throw new Error("missing model filter panel trigger");
    }

    act(() => {
      panelTrigger.click();
    });

    const responseTarget = document.body.querySelector(
      '[data-testid="model-filter-field-target-response"]',
    );
    const reroutedOnlyButton = document.body.querySelector(
      '[data-testid="model-filter-field-rerouted-only"]',
    );
    const modelTrigger = document.body.querySelector('button[role="combobox"][aria-label="Model"]');
    if (!(responseTarget instanceof HTMLButtonElement)) {
      throw new Error("missing response target");
    }
    if (!(reroutedOnlyButton instanceof HTMLButtonElement)) {
      throw new Error("missing rerouted button");
    }
    if (!(modelTrigger instanceof HTMLButtonElement)) {
      throw new Error("missing model trigger");
    }

    act(() => {
      responseTarget.click();
    });
    expect(responseTarget.dataset.active).toBe("true");

    act(() => {
      reroutedOnlyButton.click();
    });
    expect(reroutedOnlyButton.dataset.active).toBe("true");

    act(() => {
      modelTrigger.click();
    });

    const modelInput = document.body.querySelector('[cmdk-input][aria-label="Model"]');
    if (!(modelInput instanceof HTMLInputElement)) {
      throw new Error("missing model input");
    }

    const reasoningTrigger = document.body.querySelector(
      'button[role="combobox"][aria-label="Reasoning effort"]',
    );
    if (!(reasoningTrigger instanceof HTMLButtonElement)) {
      throw new Error("missing reasoning trigger");
    }

    act(() => {
      reasoningTrigger.click();
    });

    const reasoningInput = document.body.querySelector(
      '[cmdk-input][aria-label="Reasoning effort"]',
    );
    if (!(reasoningInput instanceof HTMLInputElement)) {
      throw new Error("missing reasoning input");
    }

    act(() => {
      setNativeValue(reasoningInput, "high");
      reasoningInput.dispatchEvent(new Event("input", { bubbles: true }));
    });

    const reasoningOption = Array.from(document.body.querySelectorAll("[cmdk-item]")).find(
      (candidate) => (candidate.textContent || "").includes("high"),
    );
    if (!(reasoningOption instanceof HTMLElement)) {
      throw new Error("missing reasoning option");
    }

    act(() => {
      reasoningOption.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    expect(field.textContent ?? "").toContain("gpt-5.4");
    expect(field.textContent ?? "").toContain("Reasoning effort high");
  });
});
