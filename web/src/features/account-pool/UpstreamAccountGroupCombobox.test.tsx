/** @vitest-environment jsdom */
import * as React from "react";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { UpstreamAccountGroupCombobox } from "./UpstreamAccountGroupCombobox";

class MockPointerEvent extends MouseEvent {
  pointerType: string;

  constructor(type: string, init: MouseEventInit & { pointerType?: string } = {}) {
    super(type, init);
    this.pointerType = init.pointerType ?? "mouse";
  }
}

class MockResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}

beforeAll(() => {
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
    configurable: true,
    writable: true,
    value: true,
  });
  Object.defineProperty(window, "PointerEvent", {
    configurable: true,
    writable: true,
    value: MockPointerEvent,
  });
  Object.defineProperty(globalThis, "PointerEvent", {
    configurable: true,
    writable: true,
    value: MockPointerEvent,
  });
  Object.defineProperty(window, "ResizeObserver", {
    configurable: true,
    writable: true,
    value: MockResizeObserver,
  });
  Object.defineProperty(globalThis, "ResizeObserver", {
    configurable: true,
    writable: true,
    value: MockResizeObserver,
  });
  Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
    configurable: true,
    writable: true,
    value: () => undefined,
  });
});

let root: Root | null = null;
let host: HTMLDivElement | null = null;

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  root = null;
  host = null;
});

function render(ui: React.ReactNode) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(ui);
  });
}

function pressButton(button: HTMLButtonElement) {
  act(() => {
    button.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    button.dispatchEvent(new PointerEvent("pointerup", { bubbles: true }));
    button.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    button.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
    button.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
}

function setFieldValue(input: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
  if (!setter) {
    throw new Error("missing native input setter");
  }
  act(() => {
    setter.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new Event("change", { bubbles: true }));
  });
}

function createHarness() {
  const onValueChangeSpy = vi.fn();

  function Harness() {
    const [value, setValue] = React.useState("");
    return (
      <UpstreamAccountGroupCombobox
        value={value}
        suggestions={["Prod", "prod"]}
        options={[
          { groupName: "Prod", accountCount: 2, isPersisted: true },
          { groupName: "prod", accountCount: 1, isPersisted: true },
        ]}
        placeholder="Select a group"
        createLabel={(nextValue) => `Configure "${nextValue}"`}
        formatAccountCountLabel={(count) => `${count} accounts`}
        onValueChange={(nextValue) => {
          onValueChangeSpy(nextValue);
          setValue(nextValue);
        }}
      />
    );
  }

  return { Harness, onValueChangeSpy };
}

describe("UpstreamAccountGroupCombobox", () => {
  it("keeps case-distinct group names as separate options and only hides create on exact matches", () => {
    const { Harness } = createHarness();
    render(<Harness />);

    const trigger = document.querySelector('button[role="combobox"]') as HTMLButtonElement;
    pressButton(trigger);

    const optionTexts = Array.from(document.querySelectorAll("[cmdk-item]")).map(
      (candidate) => candidate.textContent?.replace(/\s+/g, " ").trim() ?? "",
    );
    expect(optionTexts.some((text) => /Prod/.test(text))).toBe(true);
    expect(optionTexts.some((text) => /prod/.test(text))).toBe(true);

    const searchInput = document.querySelector("[cmdk-input]") as HTMLInputElement;
    setFieldValue(searchInput, "PROD");
    expect(document.body.textContent).toContain('Configure "PROD"');

    setFieldValue(searchInput, "Prod");
    expect(document.body.textContent).not.toContain('Configure "Prod"');
  });
});
