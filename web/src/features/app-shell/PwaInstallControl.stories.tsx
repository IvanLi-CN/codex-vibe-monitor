import type { Meta, StoryObj } from "@storybook/react-vite";
import { useLayoutEffect } from "react";
import { PwaInstallControl } from "./PwaInstallControl";

const labels = {
  promptButton: "Install app",
  manualButton: "Add to Home Screen",
  installedButton: "Installed",
  switcherAria: "Open install app controls",
  closeButton: "Close",
  closeAria: "Close install dialog",
  shellReady: "Offline shell ready",
  shellPending: "Offline shell pending",
  offlineChip: "Offline now",
  manualTitle: "Add Codex Vibe Monitor to Home Screen",
  manualDescription:
    "Safari on iPhone and iPad uses the browser share sheet instead of a native install prompt.",
  manualStepOpenShare: "Open Safari’s share sheet while this workspace is visible.",
  manualStepAdd: "Choose “Add to Home Screen” from the action list.",
  manualStepConfirm: "Confirm the icon name, then launch the installed app from your Home Screen.",
  installedTitle: "App already installed",
  installedDescription:
    "This browser is already running inside the installed Codex Vibe Monitor app shell.",
  installedHint:
    "Pin the installed window for daily monitoring. The app shell stays available offline, but live proxy data resumes only after reconnect.",
};

function ThemeRoot({
  children,
  theme,
}: {
  children: React.ReactNode;
  theme: "vibe-light" | "vibe-dark";
}) {
  useLayoutEffect(() => {
    const previousTheme = document.documentElement.getAttribute("data-theme");
    document.documentElement.setAttribute("data-theme", theme);
    return () => {
      if (previousTheme) {
        document.documentElement.setAttribute("data-theme", previousTheme);
      } else {
        document.documentElement.removeAttribute("data-theme");
      }
    };
  }, [theme]);

  return (
    <div
      data-theme={theme}
      className="min-h-[16rem] rounded-3xl border border-base-300/70 bg-base-100/85 p-6 shadow-[0_18px_40px_rgba(15,23,42,0.09)]"
    >
      <div className="flex items-center justify-between gap-4">
        <div className="space-y-2">
          <p className="text-xs font-semibold uppercase tracking-[0.18em] text-primary/72">
            Install surface
          </p>
          <h2 className="section-title text-xl">Header install affordance</h2>
          <p className="max-w-[52ch] text-sm text-base-content/72">
            Prompt, Safari manual guidance, and installed-state vocabulary all share the same shell
            component.
          </p>
        </div>
        {children}
      </div>
    </div>
  );
}

const meta = {
  title: "Shell/PWA Install Control",
  component: PwaInstallControl,
  tags: ["autodocs"],
  args: {
    labels,
    shellReady: true,
    isOffline: false,
    onPromptInstall: () => undefined,
  },
  render: (args) => (
    <ThemeRoot theme="vibe-light">
      <PwaInstallControl {...args} />
    </ThemeRoot>
  ),
} satisfies Meta<typeof PwaInstallControl>;

export default meta;

type Story = StoryObj<typeof meta>;

export const Promptable: Story = {
  args: {
    mode: "prompt",
  },
};

export const SafariManual: Story = {
  args: {
    mode: "manual-ios",
  },
};

export const InstalledOffline: Story = {
  args: {
    mode: "installed",
    isOffline: true,
  },
  render: (args) => (
    <ThemeRoot theme="vibe-dark">
      <PwaInstallControl {...args} />
    </ThemeRoot>
  ),
};
