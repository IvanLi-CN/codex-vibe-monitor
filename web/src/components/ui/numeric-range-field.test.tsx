/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { NumericRangeField } from "./numeric-range-field";

let host: HTMLDivElement | null = null;
let root: Root | null = null;

beforeAll(() => {
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
    configurable: true,
    writable: true,
    value: true,
  });
  if (typeof globalThis.PointerEvent === "undefined") {
    Object.defineProperty(window, "PointerEvent", {
      configurable: true,
      writable: true,
      value: MouseEvent,
    });
    Object.defineProperty(globalThis, "PointerEvent", {
      configurable: true,
      writable: true,
      value: MouseEvent,
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

describe("NumericRangeField", () => {
  it("updates both ends through a single field wrapper", () => {
    const onChange = vi.fn();

    render(
      <NumericRangeField
        label="Total tokens"
        sliderMin={0}
        sliderMax={200}
        minAriaLabel="Minimum total tokens"
        maxAriaLabel="Maximum total tokens"
        minValue="10"
        maxValue="20"
        onChange={onChange}
      />,
    );
    expect(document.querySelector('input[aria-label="Minimum total tokens"]')).toBeNull();
    expect(document.querySelector('input[aria-label="Maximum total tokens"]')).toBeNull();
  });

  it("updates the minimum bound from the slider", () => {
    const onChange = vi.fn();

    render(
      <NumericRangeField
        label="Total duration"
        sliderMin={0}
        sliderMax={5000}
        minAriaLabel="Minimum total duration"
        maxAriaLabel="Maximum total duration"
        minValue=""
        maxValue=""
        step={0.1}
        onChange={onChange}
      />,
    );

    const minSlider = document.querySelector('input[aria-label="Minimum total duration slider"]');
    if (!(minSlider instanceof HTMLInputElement)) {
      throw new Error("missing minimum slider");
    }

    act(() => {
      setNativeValue(minSlider, "3200");
      minSlider.dispatchEvent(new Event("input", { bubbles: true }));
    });

    expect(onChange).toHaveBeenCalledWith({ minValue: "3200", maxValue: "" });
  });

  it("exposes invalid state and current selection summary", () => {
    render(
      <NumericRangeField
        label="Total tokens"
        sliderMin={0}
        sliderMax={12000}
        minAriaLabel="Minimum total tokens"
        maxAriaLabel="Maximum total tokens"
        unitLabel="TOKENS"
        minValue="2400"
        maxValue="6400"
        error="Total tokens range must be in ascending order."
        onChange={() => {}}
      />,
    );

    const minSlider = document.querySelector('input[aria-label="Minimum total tokens slider"]');
    const errorBubble = document.querySelector('[role="alert"]');
    if (!(minSlider instanceof HTMLInputElement)) {
      throw new Error("missing minimum slider");
    }
    if (!(errorBubble instanceof HTMLElement)) {
      throw new Error("missing error bubble");
    }

    expect(minSlider.getAttribute("aria-invalid")).toBe("true");
    expect(minSlider.getAttribute("aria-describedby")).toBe(errorBubble.id);
    expect(document.body.textContent ?? "").toContain("2,400 - 6,400 TOKENS");
  });

  it("updates the nearest thumb when the track is pressed", () => {
    const onChange = vi.fn();

    render(
      <NumericRangeField
        label="Total duration"
        sliderMin={0}
        sliderMax={5000}
        minAriaLabel="Minimum total duration"
        maxAriaLabel="Maximum total duration"
        unitLabel="MS"
        minValue="1000"
        maxValue="4000"
        onChange={onChange}
      />,
    );

    const slider = document.querySelector(".dual-range-slider");
    if (!(slider instanceof HTMLDivElement)) {
      throw new Error("missing slider surface");
    }
    slider.getBoundingClientRect = () =>
      ({
        x: 0,
        y: 0,
        width: 100,
        height: 44,
        top: 0,
        left: 0,
        right: 100,
        bottom: 44,
        toJSON: () => "",
      }) as DOMRect;

    act(() => {
      slider.dispatchEvent(
        new PointerEvent("pointerdown", {
          bubbles: true,
          clientX: 70,
        }),
      );
    });

    expect(onChange).toHaveBeenCalledWith({ minValue: "1000", maxValue: "3500" });
  });

  it("supports an embedded surface without rendering a nested card shell", () => {
    render(
      <NumericRangeField
        label="Total tokens"
        testId="embedded-range"
        surface="embedded"
        sliderMin={0}
        sliderMax={12000}
        minAriaLabel="Minimum total tokens"
        maxAriaLabel="Maximum total tokens"
        unitLabel="TOKENS"
        minValue="2400"
        maxValue="6400"
        onChange={() => {}}
      />,
    );

    const field = document.querySelector('[data-testid="embedded-range"]');
    if (!(field instanceof HTMLDivElement)) {
      throw new Error("missing embedded field");
    }
    const chrome = field.children.item(1);
    if (!(chrome instanceof HTMLDivElement)) {
      throw new Error("missing embedded chrome");
    }

    expect(chrome.className).not.toContain("border");
    expect(chrome.className).not.toContain("bg-base-100");
  });
});
