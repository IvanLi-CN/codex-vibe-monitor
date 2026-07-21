/** @vitest-environment jsdom */
import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { MultiValueSuggestionField } from "./multi-value-suggestion-field";
import { OverlayHostProvider } from "./overlay-host";

let host: HTMLDivElement | null = null;
let root: Root | null = null;
let overlayHost: HTMLDivElement | null = null;

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
  overlayHost?.remove();
  host = null;
  root = null;
  overlayHost = null;
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

function ControlledField({
  label,
  inputLabel,
  options,
  initialValues = [],
  surface,
}: {
  label: string;
  inputLabel: string;
  options: Array<string | { value: string; label?: string; searchText?: string }>;
  initialValues?: string[];
  surface?: "default" | "embedded";
}) {
  const [values, setValues] = useState(initialValues);
  const [inputValue, setInputValue] = useState("");

  return (
    <MultiValueSuggestionField
      label={label}
      inputLabel={inputLabel}
      id="multi-value-test-input"
      values={values}
      onValuesChange={setValues}
      inputValue={inputValue}
      onInputValueChange={setInputValue}
      options={options}
      addLabel="Add"
      placeholder="Search"
      emptyText="No matches"
      surface={surface}
    />
  );
}

describe("MultiValueSuggestionField", () => {
  it("selects and toggles values through the tag selector trigger", () => {
    render(
      <ControlledField label="Model" inputLabel="Model" options={["gpt-5.4", "gpt-5-mini"]} />,
    );

    const trigger = document.querySelector('button[role="combobox"][aria-label="Model"]');
    if (!(trigger instanceof HTMLButtonElement)) {
      throw new Error("missing multi-value trigger");
    }

    act(() => {
      trigger.click();
    });

    const input = document.querySelector('[cmdk-input][aria-label="Model"]');
    if (!(input instanceof HTMLInputElement)) {
      throw new Error("missing multi-value input");
    }

    act(() => {
      setNativeValue(input, "gpt-5.4");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });

    const option = Array.from(document.body.querySelectorAll("[cmdk-item]")).find((candidate) =>
      (candidate.textContent || "").includes("gpt-5.4"),
    );
    if (!(option instanceof HTMLElement)) {
      throw new Error("missing command item");
    }

    act(() => {
      option.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    expect(trigger.textContent ?? "").toContain("gpt-5.4");

    const selectedOption = Array.from(document.body.querySelectorAll("[cmdk-item]")).find(
      (candidate) => (candidate.textContent || "").includes("gpt-5.4"),
    );
    if (!(selectedOption instanceof HTMLElement)) {
      throw new Error("missing selected command item");
    }

    act(() => {
      selectedOption.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    expect(trigger.textContent ?? "").not.toContain("gpt-5.4");
  });

  it("renders option labels for selected values", () => {
    render(
      <ControlledField
        label="Upstream account"
        inputLabel="Upstream account"
        initialValues={["42"]}
        options={[
          {
            value: "42",
            label: "Pool Alpha (#42)",
            searchText: "Pool Alpha 42",
          },
        ]}
      />,
    );

    expect(document.body.textContent ?? "").toContain("Pool Alpha (#42)");
    expect(
      document.querySelector('button[role="combobox"][aria-label="Upstream account"]'),
    ).not.toBeNull();
  });

  it("renders the embedded surface as a single field with a floating selector", () => {
    render(
      <ControlledField
        label="Model"
        inputLabel="Model"
        surface="embedded"
        options={["gpt-5.4", "gpt-5-mini"]}
      />,
    );

    const field = document.querySelector(".field");
    const trigger = document.querySelector('button[role="combobox"][aria-label="Model"]');
    if (!(field instanceof HTMLDivElement)) {
      throw new Error("missing field");
    }
    if (!(trigger instanceof HTMLButtonElement)) {
      throw new Error("missing embedded trigger");
    }

    act(() => {
      trigger.click();
    });

    const floatingInput = document.body.querySelector('[cmdk-input][aria-label="Model"]');
    const floatingOption = document.body.querySelector("[cmdk-item]");
    expect(floatingInput).not.toBeNull();
    expect(floatingOption).not.toBeNull();
    expect(field.querySelector('[cmdk-input][aria-label="Model"]')).toBeNull();
    expect(document.body.querySelector("[data-radix-popper-content-wrapper]")).not.toBeNull();
  });

  it("keeps the embedded selector inside the inherited overlay host", () => {
    overlayHost = document.createElement("div");
    overlayHost.setAttribute("data-testid", "overlay-host");
    document.body.appendChild(overlayHost);

    render(
      <OverlayHostProvider value={overlayHost}>
        <ControlledField
          label="Model"
          inputLabel="Model"
          surface="embedded"
          options={["gpt-5.4", "gpt-5-mini"]}
        />
      </OverlayHostProvider>,
    );

    const trigger = document.querySelector('button[role="combobox"][aria-label="Model"]');
    if (!(trigger instanceof HTMLButtonElement)) {
      throw new Error("missing embedded trigger");
    }

    act(() => {
      trigger.click();
    });

    const floatingInput = document.body.querySelector('[cmdk-input][aria-label="Model"]');
    const floatingWrapper = document.body.querySelector("[data-radix-popper-content-wrapper]");

    expect(floatingInput).not.toBeNull();
    expect(floatingWrapper).not.toBeNull();
    expect(overlayHost.contains(floatingWrapper)).toBe(true);
    expect(host?.contains(floatingWrapper)).toBe(false);
  });
});
