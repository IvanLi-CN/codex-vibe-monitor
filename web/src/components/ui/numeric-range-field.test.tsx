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

    const minSlider = document.querySelector(
      '[role="slider"][aria-label="Minimum total duration slider"]',
    );
    if (!(minSlider instanceof HTMLElement)) {
      throw new Error("missing minimum slider");
    }

    act(() => {
      minSlider.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "ArrowRight" }));
    });

    expect(onChange).toHaveBeenCalledWith({ minValue: "0.1", maxValue: "" });
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

    const minSlider = document.querySelector(
      '[role="slider"][aria-label="Minimum total tokens slider"]',
    );
    const errorBubble = document.querySelector('[role="alert"]');
    if (!(minSlider instanceof HTMLElement)) {
      throw new Error("missing minimum slider");
    }
    if (!(errorBubble instanceof HTMLElement)) {
      throw new Error("missing error bubble");
    }

    expect(minSlider.getAttribute("aria-invalid")).toBe("true");
    expect(minSlider.getAttribute("aria-describedby")).toBe(errorBubble.id);
    expect(document.body.textContent ?? "").toContain("2,400 - 6,400 TOKENS");
  });

  it("renders slider thumbs that stay aligned to the same shared track", () => {
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
        testId="duration-range"
      />,
    );

    const slider = document.querySelector('[data-testid="duration-range-slider"]');
    if (!(slider instanceof HTMLElement)) {
      throw new Error("missing slider surface");
    }
    const sliders = Array.from(slider.querySelectorAll('[role="slider"]'));
    const track = slider.querySelector('[data-slot="slider-track"]');
    expect(sliders).toHaveLength(2);
    expect(track).not.toBeNull();
    expect(onChange).not.toHaveBeenCalled();
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
