/** @vitest-environment jsdom */
import { act, useRef } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { usePointerTransitionGuard } from "./usePointerTransitionGuard";

let host: HTMLDivElement | null = null;
let root: Root | null = null;
const onClose = vi.fn();

beforeAll(() => {
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
    configurable: true,
    writable: true,
    value: true,
  });
});

afterEach(() => {
  vi.useRealTimers();
  act(() => root?.unmount());
  host?.remove();
  host = null;
  root = null;
  onClose.mockReset();
});

function Harness() {
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const contentRef = useRef<HTMLDivElement | null>(null);
  const transition = usePointerTransitionGuard({
    triggerRef,
    contentRef,
    onClose,
  });

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        onPointerLeave={(event) => transition.start("trigger", event)}
      >
        Trigger
      </button>
      <div ref={contentRef} onPointerEnter={transition.cancel}>
        Content
      </div>
    </>
  );
}

function render() {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => root?.render(<Harness />));

  const trigger = host.querySelector("button") as HTMLButtonElement;
  const content = host.querySelector("div") as HTMLDivElement;
  Object.defineProperty(trigger, "getBoundingClientRect", {
    configurable: true,
    value: () => ({ left: 100, right: 120, top: 100, bottom: 120 }),
  });
  Object.defineProperty(content, "getBoundingClientRect", {
    configurable: true,
    value: () => ({ left: 150, right: 250, top: 20, bottom: 60 }),
  });
  return { trigger, content };
}

function leaveTrigger(trigger: HTMLButtonElement) {
  trigger.dispatchEvent(
    new PointerEvent("pointerout", {
      bubbles: true,
      pointerType: "mouse",
      clientX: 110,
      clientY: 100,
    }),
  );
}

describe("usePointerTransitionGuard", () => {
  it("keeps a slow crossing open, closes outside its corridor, and closes after 500ms at rest", () => {
    vi.useFakeTimers();
    const { trigger, content } = render();

    act(() => {
      leaveTrigger(trigger);
      document.dispatchEvent(
        new PointerEvent("pointermove", {
          bubbles: true,
          pointerType: "mouse",
          clientX: 130,
          clientY: 80,
        }),
      );
      vi.advanceTimersByTime(300);
    });
    expect(onClose).not.toHaveBeenCalled();

    act(() => {
      content.dispatchEvent(
        new PointerEvent("pointerover", { bubbles: true, pointerType: "mouse" }),
      );
      vi.advanceTimersByTime(600);
    });
    expect(onClose).not.toHaveBeenCalled();

    act(() => {
      leaveTrigger(trigger);
      document.dispatchEvent(
        new PointerEvent("pointermove", {
          bubbles: true,
          pointerType: "mouse",
          clientX: 320,
          clientY: 250,
        }),
      );
    });
    expect(onClose).toHaveBeenCalledTimes(1);

    act(() => {
      leaveTrigger(trigger);
      vi.advanceTimersByTime(499);
    });
    expect(onClose).toHaveBeenCalledTimes(1);

    act(() => vi.advanceTimersByTime(1));
    expect(onClose).toHaveBeenCalledTimes(2);
  });
});
