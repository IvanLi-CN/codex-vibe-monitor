/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { Tooltip } from "./tooltip";

let host: HTMLDivElement | null = null;
let root: Root | null = null;

beforeAll(() => {
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
    configurable: true,
    writable: true,
    value: true,
  });
  if (!("ResizeObserver" in globalThis)) {
    Object.defineProperty(globalThis, "ResizeObserver", {
      configurable: true,
      writable: true,
      value: class ResizeObserver {
        observe() {}
        unobserve() {}
        disconnect() {}
      },
    });
  }
});

afterEach(() => {
  vi.useRealTimers();
  act(() => root?.unmount());
  host?.remove();
  host = null;
  root = null;
});

function render(ui: React.ReactNode) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => root?.render(ui));
}

describe("Tooltip", () => {
  it("keeps its content mounted while the pointer crosses from trigger to content", () => {
    vi.useFakeTimers();
    render(
      <Tooltip content={<span>Readable details</span>}>
        <button type="button">Details</button>
      </Tooltip>,
    );

    const trigger = host?.querySelector("button");
    expect(trigger).toBeInstanceOf(HTMLButtonElement);
    act(() => trigger?.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })));

    const content = document.body.querySelector('[data-side][data-state="instant-open"]');
    expect(content).toBeInstanceOf(HTMLElement);

    act(() => {
      trigger?.dispatchEvent(
        new PointerEvent("pointerout", { bubbles: true, clientX: 0, clientY: 0 }),
      );
      trigger?.dispatchEvent(
        new PointerEvent("pointerleave", { bubbles: true, clientX: 0, clientY: 0 }),
      );
      trigger?.dispatchEvent(new MouseEvent("mouseout", { bubbles: true, relatedTarget: null }));
      vi.advanceTimersByTime(300);
    });
    expect(document.body.querySelector('[data-side][data-state="instant-open"]')).toBe(content);

    act(() => {
      content?.dispatchEvent(
        new PointerEvent("pointerover", { bubbles: true, clientX: 0, clientY: 0 }),
      );
      content?.dispatchEvent(
        new PointerEvent("pointerenter", { bubbles: true, clientX: 0, clientY: 0 }),
      );
      content?.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
      vi.advanceTimersByTime(300);
    });
    expect(document.body.querySelector('[data-side][data-state="instant-open"]')).toBe(content);
  });

  it("keeps a click-pinned tooltip open after pointer exit and closes on Escape or outside press", () => {
    vi.useFakeTimers();
    render(
      <>
        <Tooltip clickToOpen content={<span>Readable details</span>}>
          <button type="button">Details</button>
        </Tooltip>
        <button type="button">Outside</button>
      </>,
    );

    const trigger = host?.querySelector("button");
    act(() => trigger?.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    act(() => trigger?.focus());
    let content = document.body.querySelector('[data-side][data-state="instant-open"]');
    expect(content).toBeInstanceOf(HTMLElement);

    act(() => {
      trigger?.dispatchEvent(
        new PointerEvent("pointerout", { bubbles: true, clientX: 0, clientY: 0 }),
      );
      trigger?.dispatchEvent(
        new PointerEvent("pointerleave", { bubbles: true, clientX: 0, clientY: 0 }),
      );
      vi.advanceTimersByTime(600);
    });
    expect(document.body.querySelector('[data-side][data-state="instant-open"]')).toBe(content);

    act(() => {
      content?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    });
    content = document.body.querySelector('[data-side][data-state="instant-open"]');
    expect(content).toBeNull();

    act(() => trigger?.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    act(() => trigger?.focus());
    content = document.body.querySelector('[data-side][data-state="instant-open"]');
    expect(content).toBeInstanceOf(HTMLElement);
    act(() => {
      const outsideButton = host?.querySelectorAll("button")[1];
      outsideButton?.dispatchEvent(
        new PointerEvent("pointerdown", {
          bubbles: true,
          button: 0,
          clientX: 400,
          clientY: 400,
          isPrimary: true,
          pointerType: "mouse",
        }),
      );
      outsideButton?.focus();
    });
    expect(document.body.querySelector('[data-side][data-state="instant-open"]')).toBeNull();
  });
});
