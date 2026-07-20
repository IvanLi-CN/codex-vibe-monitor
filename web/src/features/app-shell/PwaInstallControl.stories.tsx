import type { Meta, StoryObj } from "@storybook/react-vite";
import { useLayoutEffect } from "react";
import { expect, within } from "storybook/test";
import { PwaInstallControl } from "./PwaInstallControl";

const labels = {
  promptButton: "Install app",
  laterButton: "Later",
  manualButton: "Add to Home Screen",
  installedButton: "Installed",
  switcherAria: "Open install app controls",
  closeButton: "Close",
  closeAria: "Close install dialog",
  shellReady: "Offline shell ready",
  shellPending: "Offline shell pending",
  offlineChip: "Offline now",
  promptTitle: "Install Codex Vibe Monitor",
  promptDescription:
    "Add this workspace as a standalone app window so daily monitoring opens directly into the shell.",
  promptHint:
    "After installation, the app shell can still reopen offline. Live proxy data resumes after the network reconnects.",
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

function ShellBackdrop({
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
      className="flex min-h-screen items-start justify-center bg-[radial-gradient(circle_at_top,rgba(56,189,248,0.16),transparent_28%),linear-gradient(180deg,rgba(248,250,252,1),rgba(237,242,247,1))] px-4 py-6"
    >
      <div className="w-[min(100%,24rem)] overflow-hidden rounded-[2rem] border border-base-300/70 bg-base-100/96 shadow-[0_24px_64px_rgba(15,23,42,0.16)]">
        <div className="border-b border-base-300/70 bg-sky-500/90 px-4 py-3 text-sm font-semibold text-white">
          Codex Vibe Monitor
        </div>
        <div className="space-y-3 px-3 py-4">
          <div className="rounded-[1.35rem] border border-base-300/70 bg-base-100/92 px-4 py-5 shadow-sm">
            <div className="h-3 w-28 rounded-full bg-base-300/75" />
            <div className="mt-3 h-3 w-full rounded-full bg-base-200/85" />
            <div className="mt-2 h-3 w-4/5 rounded-full bg-base-200/85" />
          </div>
          <div className="rounded-[1.35rem] border border-base-300/70 bg-base-100/92 px-4 py-5 shadow-sm">
            <div className="h-3 w-24 rounded-full bg-base-300/75" />
            <div className="mt-3 h-3 w-full rounded-full bg-base-200/85" />
            <div className="mt-2 h-3 w-3/5 rounded-full bg-base-200/85" />
          </div>
        </div>
      </div>
      {children}
    </div>
  );
}

const meta = {
  title: "Shell/PWA Install Dialog",
  component: PwaInstallControl,
  tags: ["autodocs"],
  args: {
    labels,
    shellReady: true,
    isOffline: false,
    open: true,
    onOpenChange: () => undefined,
    onPromptInstall: () => undefined,
  },
  parameters: {
    layout: "fullscreen",
  },
} satisfies Meta<typeof PwaInstallControl>;

export default meta;

type Story = StoryObj<typeof meta>;

export const PromptDialogMobile: Story = {
  args: {
    mode: "prompt",
  },
  render: (args) => (
    <ShellBackdrop theme="vibe-light">
      <PwaInstallControl {...args} />
    </ShellBackdrop>
  ),
  play: async () => {
    await expect(within(document.body).getByRole("dialog")).toBeInTheDocument();
    await expect(within(document.body).getByText("Install Codex Vibe Monitor")).toBeInTheDocument();
  },
};

export const SafariManualMobile: Story = {
  args: {
    mode: "manual-ios",
  },
  render: (args) => (
    <ShellBackdrop theme="vibe-light">
      <PwaInstallControl {...args} />
    </ShellBackdrop>
  ),
};

export const InstalledSummaryDark: Story = {
  args: {
    mode: "installed",
    isOffline: true,
  },
  render: (args) => (
    <ShellBackdrop theme="vibe-dark">
      <PwaInstallControl {...args} />
    </ShellBackdrop>
  ),
  play: async () => {
    await expect(within(document.body).getByRole("dialog")).toBeInTheDocument();
    await expect(within(document.body).getByText("Offline now")).toBeInTheDocument();
    await expect(within(document.body).getByText("App already installed")).toBeInTheDocument();
  },
};
