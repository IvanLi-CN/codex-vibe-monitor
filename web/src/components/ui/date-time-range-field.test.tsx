/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { DateTimeRangeField } from "./date-time-range-field";

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

describe("DateTimeRangeField", () => {
  it("switches into custom mode when editing date-time inputs", async () => {
    const onChange = vi.fn();

    render(
      <DateTimeRangeField
        label="Time range"
        customPresetValue="custom"
        value={{ preset: "today", from: "2026-07-18T00:00", to: "2026-07-18T12:00" }}
        options={[
          { value: "today", label: "Today" },
          { value: "custom", label: "Custom range" },
        ]}
        fromName="customFrom"
        toName="customTo"
        onChange={onChange}
      />,
    );

    const trigger = document.querySelector("button");
    if (!(trigger instanceof HTMLButtonElement)) {
      throw new Error("missing trigger");
    }

    await act(async () => {
      trigger.click();
      await Promise.resolve();
    });

    const input = document.body.querySelector('input[name="customFrom"]');
    if (!(input instanceof HTMLInputElement)) {
      throw new Error("missing custom from input");
    }

    act(() => {
      setNativeValue(input, "2026-07-18T08:00");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });

    expect(onChange).toHaveBeenCalledWith({
      preset: "custom",
      from: "2026-07-18T08:00",
      to: "2026-07-18T12:00",
    });
  });

  it("binds the field error to the trigger and custom inputs", async () => {
    render(
      <DateTimeRangeField
        label="Time range"
        customPresetValue="custom"
        value={{ preset: "custom", from: "2026-07-18T15:00", to: "2026-07-18T14:00" }}
        options={[
          { value: "today", label: "Today" },
          { value: "custom", label: "Custom range" },
        ]}
        fromName="customFrom"
        toName="customTo"
        error="End time must be after start time."
        onChange={() => {}}
      />,
    );

    const trigger = document.querySelector("button");
    const errorBubble = document.querySelector('[role="alert"]');
    if (!(trigger instanceof HTMLButtonElement)) {
      throw new Error("missing trigger");
    }
    if (!(errorBubble instanceof HTMLElement)) {
      throw new Error("missing error bubble");
    }

    expect(trigger.getAttribute("aria-invalid")).toBe("true");
    expect(trigger.getAttribute("aria-describedby")).toBe(errorBubble.id);

    await act(async () => {
      trigger.click();
      await Promise.resolve();
    });

    const fromInput = document.body.querySelector('input[name="customFrom"]');
    if (!(fromInput instanceof HTMLInputElement)) {
      throw new Error("missing custom from input");
    }
    expect(fromInput.getAttribute("aria-invalid")).toBe("true");
    expect(fromInput.getAttribute("aria-describedby")).toBe(errorBubble.id);
  });
});
