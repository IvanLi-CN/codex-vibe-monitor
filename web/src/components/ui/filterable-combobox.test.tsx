/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { FilterableCombobox } from "./filterable-combobox";

let host: HTMLDivElement | null = null;
let root: Root | null = null;

beforeAll(() => {
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
    configurable: true,
    writable: true,
    value: true,
  });
});

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
  vi.restoreAllMocks();
});

function render(ui: React.ReactNode) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(ui);
  });
}

function getComboboxInput() {
  const input = document.querySelector('input[role="combobox"]');
  if (!(input instanceof HTMLInputElement)) {
    throw new Error("missing combobox input");
  }
  return input;
}

describe("FilterableCombobox", () => {
  it("disables browser native autocomplete hints by default", () => {
    render(
      <FilterableCombobox
        label="Model"
        name="model"
        value=""
        onValueChange={() => {}}
        options={["gpt-5.4", "deepseek-v3.1"]}
      />,
    );

    const input = getComboboxInput();
    expect(input.autocomplete).toBe("off");
    expect(input.getAttribute("autocorrect")).toBe("off");
    expect(input.getAttribute("autocapitalize")).toBe("none");
    expect(input.getAttribute("spellcheck")).toBe("false");
  });

  it("allows explicit autocomplete overrides when a caller needs them", () => {
    render(
      <FilterableCombobox
        label="Model"
        name="model"
        value=""
        onValueChange={() => {}}
        options={["gpt-5.4", "deepseek-v3.1"]}
        inputAutocompleteProps={{
          autoComplete: "on",
          autoCorrect: "on",
          autoCapitalize: "sentences",
          spellCheck: true,
        }}
      />,
    );

    const input = getComboboxInput();
    expect(input.autocomplete).toBe("on");
    expect(input.getAttribute("autocorrect")).toBe("on");
    expect(input.getAttribute("autocapitalize")).toBe("sentences");
    expect(input.getAttribute("spellcheck")).toBe("true");
  });

  it("uses option labels for display and returns the selected option", () => {
    const onValueChange = vi.fn();
    const onOptionSelect = vi.fn();

    render(
      <FilterableCombobox
        label="Account"
        name="account"
        value=""
        onValueChange={onValueChange}
        onOptionSelect={onOptionSelect}
        options={[
          { value: "42", label: "Pool Alpha (#42)", searchText: "Pool Alpha 42" },
          { value: "77", label: "Pool Beta (#77)", searchText: "Pool Beta 77" },
        ]}
      />,
    );

    const input = getComboboxInput();
    act(() => {
      input.focus();
      input.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    const option = Array.from(document.querySelectorAll('button[role="option"]')).find(
      (candidate) => candidate.textContent?.includes("Pool Alpha (#42)"),
    );
    if (!(option instanceof HTMLButtonElement)) {
      throw new Error("missing labeled option");
    }

    act(() => {
      option.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    });

    expect(onValueChange).toHaveBeenCalledWith("Pool Alpha (#42)");
    expect(onOptionSelect).toHaveBeenCalledWith({
      value: "42",
      label: "Pool Alpha (#42)",
      searchText: "Pool Alpha 42",
    });
  });
});
