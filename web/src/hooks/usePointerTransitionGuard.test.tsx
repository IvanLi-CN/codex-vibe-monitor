/** @vitest-environment jsdom */
import { act, createRef, useRef } from "react";
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

function Harness({ revision = 0 }: { revision?: number }) {
  const refsRef = useRef<{
    revision: number;
    triggerRef: ReturnType<typeof createRef<HTMLButtonElement>>;
    contentRef: ReturnType<typeof createRef<HTMLDivElement>>;
  } | null>(null);
  if (!refsRef.current || refsRef.current.revision !== revision) {
    refsRef.current = {
      revision,
      triggerRef: createRef<HTMLButtonElement>(),
      contentRef: createRef<HTMLDivElement>(),
    };
  }
  const { triggerRef, contentRef } = refsRef.current;
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

  const renderRevision = (revision: number) => {
    act(() => root?.render(<Harness revision={revision} />));

    const trigger = host?.querySelector("button") as HTMLButtonElement;
    const content = host?.querySelector("div") as HTMLDivElement;
    Object.defineProperty(trigger, "getBoundingClientRect", {
      configurable: true,
      value: () => ({ left: 100, right: 120, top: 100, bottom: 120 }),
    });
    Object.defineProperty(content, "getBoundingClientRect", {
      configurable: true,
      value: () => ({ left: 150, right: 250, top: 20, bottom: 60 }),
    });
    return { trigger, content };
  };

  return { ...renderRevision(0), rerender: renderRevision };
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

  it("removes the exact pointermove listener when ref objects change during a transition", () => {
    vi.useFakeTimers();
    const addListener = vi.spyOn(document, "addEventListener");
    const removeListener = vi.spyOn(document, "removeEventListener");
    const { trigger, rerender } = render();

    act(() => leaveTrigger(trigger));

    const registration = addListener.mock.calls.find(([type]) => type === "pointermove");
    expect(registration).toBeDefined();

    const { content } = rerender(1);
    act(() => {
      content.dispatchEvent(
        new PointerEvent("pointermove", {
          bubbles: true,
          pointerType: "mouse",
          clientX: 160,
          clientY: 40,
        }),
      );
      vi.advanceTimersByTime(600);
    });

    const removal = removeListener.mock.calls.filter(([type]) => type === "pointermove").at(-1);
    expect(onClose).not.toHaveBeenCalled();
    expect(removal?.[1]).toBe(registration?.[1]);
  });
});
