/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { BatchOauthActionButton } from "./BatchOauthActionButton";

let host: HTMLDivElement | null = null;
let root: Root | null = null;

beforeAll(() => {
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
    configurable: true,
    writable: true,
    value: true,
  });
  class ResizeObserverMock {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  Object.defineProperty(globalThis, "ResizeObserver", {
    configurable: true,
    writable: true,
    value: ResizeObserverMock,
  });
  Object.defineProperty(window, "ResizeObserver", {
    configurable: true,
    writable: true,
    value: ResizeObserverMock,
  });
  if (typeof globalThis.PointerEvent === "undefined") {
    Object.defineProperty(globalThis, "PointerEvent", {
      configurable: true,
      writable: true,
      value: MouseEvent,
    });
  }
});

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
  vi.useRealTimers();
});

function render(ui: React.ReactNode) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(ui);
  });
}

function getButton(name: RegExp) {
  const button = Array.from(document.body.querySelectorAll("button")).find(
    (candidate) =>
      candidate instanceof HTMLButtonElement &&
      name.test(
        [
          candidate.textContent,
          candidate.getAttribute("aria-label"),
          candidate.getAttribute("title"),
        ]
          .filter(Boolean)
          .join(" "),
      ),
  );
  expect(button).toBeInstanceOf(HTMLButtonElement);
  return button as HTMLButtonElement;
}

describe("BatchOauthActionButton", () => {
  const baseProps = {
    primaryAriaLabel: "Copy OAuth URL",
    regenerateAriaLabel: "Regenerate OAuth URL",
    popoverTitle: "Copy OAuth URL",
    popoverDescription:
      "Copy the generated login URL, open it in the browser that will complete the login, and return here with the callback URL.",
    remainingLabel: "Current link expires in 14:59.",
    expiresAtLabel: "Expires at 2026-03-26 18:00:00.",
    manualCopyTitle: "Copy manually",
    manualCopyDescription: "Clipboard access failed.",
    onPrimaryAction: vi.fn(),
    onRegenerate: vi.fn(),
  };

  it("opens the copy popover on right click and shows countdown details", () => {
    render(<BatchOauthActionButton mode="copy" {...baseProps} />);

    const button = getButton(/copy oauth url/i);
    act(() => {
      button.dispatchEvent(
        new MouseEvent("contextmenu", {
          bubbles: true,
          cancelable: true,
        }),
      );
    });

    expect(document.body.textContent).toContain("Current link expires in 14:59.");
    expect(document.body.textContent).toContain("Expires at 2026-03-26 18:00:00.");
    expect(document.body.textContent).toContain("Regenerate OAuth URL");
  });

  it("opens on touch long press without triggering the primary click", () => {
    const onPrimaryAction = vi.fn();
    render(
      <BatchOauthActionButton
        mode="copy"
        {...baseProps}
        onPrimaryAction={onPrimaryAction}
      />,
    );

    const button = getButton(/copy oauth url/i);
    act(() => {
      button.dispatchEvent(
        new PointerEvent("pointerdown", {
          bubbles: true,
          pointerType: "touch",
          button: 0,
        }),
      );
      vi.advanceTimersByTime(430);
    });

    expect(document.body.textContent).toContain("Regenerate OAuth URL");

    act(() => {
      button.dispatchEvent(
        new PointerEvent("pointerup", {
          bubbles: true,
          pointerType: "touch",
          button: 0,
        }),
      );
      button.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    expect(onPrimaryAction).not.toHaveBeenCalled();

    act(() => {
      vi.runAllTimers();
      button.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    expect(onPrimaryAction).toHaveBeenCalledTimes(1);
  });

  it("shows the manual copy fallback and allows dismissing it", () => {
    const onManualCopyOpenChange = vi.fn();
    render(
      <BatchOauthActionButton
        mode="copy"
        {...baseProps}
        manualCopyValue="https://auth.openai.com/authorize?login=manual"
        onManualCopyOpenChange={onManualCopyOpenChange}
      />,
    );

    expect(document.body.textContent).toContain("Copy manually");
    expect(document.body.textContent).toContain(
      "https://auth.openai.com/authorize?login=manual",
    );

    act(() => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });

    expect(onManualCopyOpenChange).toHaveBeenCalledWith(false);
  });

  it("lets keyboard users open the pinned popover action with ArrowDown", () => {
    render(<BatchOauthActionButton mode="copy" {...baseProps} />);

    const button = getButton(/copy oauth url/i);
    act(() => {
      button.focus();
    });

    act(() => {
      button.dispatchEvent(
        new KeyboardEvent("keydown", {
          bubbles: true,
          key: "ArrowDown",
        }),
      );
      vi.runAllTimers();
    });

    const regenerateButton = getButton(/regenerate oauth url/i);
    expect(document.activeElement).toBe(regenerateButton);
  });

  it("does not hijack forward tab navigation from the trigger", () => {
    render(<BatchOauthActionButton mode="copy" {...baseProps} />);

    const button = getButton(/copy oauth url/i);
    act(() => {
      button.focus();
    });

    const event = new KeyboardEvent("keydown", {
      bubbles: true,
      cancelable: true,
      key: "Tab",
    });

    let dispatchResult = true;
    act(() => {
      dispatchResult = button.dispatchEvent(event);
      vi.runAllTimers();
    });

    expect(dispatchResult).toBe(true);
    expect(event.defaultPrevented).toBe(false);
    expect(document.activeElement).toBe(button);
  });
});
