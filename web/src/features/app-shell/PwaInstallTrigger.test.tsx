/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { PwaInstallTrigger } from "./PwaInstallTrigger";

describe("PwaInstallTrigger", () => {
  it("renders a labelled desktop entry and calls the opener", () => {
    const onClick = vi.fn();
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root = createRoot(host);
    act(() => {
      root.render(
        <PwaInstallTrigger
          mode="prompt"
          label="Install app"
          ariaLabel="Open install app controls"
          onClick={onClick}
        />,
      );
    });
    const button = host.querySelector("button") as HTMLButtonElement;
    expect(button.textContent).toContain("Install app");
    button.click();
    expect(onClick).toHaveBeenCalledTimes(1);
    act(() => root.unmount());
    host.remove();
  });

  it("keeps the mobile trigger icon-only while retaining an accessible label", () => {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root = createRoot(host);
    act(() => {
      root.render(
        <PwaInstallTrigger
          mode="manual-ios"
          label="Add to Home Screen"
          ariaLabel="Open install app controls"
          compact
          onClick={() => undefined}
        />,
      );
    });
    const button = host.querySelector("button") as HTMLButtonElement;
    expect(button.querySelector(".sr-only")?.textContent).toBe("Add to Home Screen");
    expect(button.title).toBe("Add to Home Screen");
    act(() => root.unmount());
    host.remove();
  });
});
