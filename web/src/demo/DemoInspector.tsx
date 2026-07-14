import { useState, useSyncExternalStore } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import { Button } from "../components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "../components/ui/dialog";
import { Tooltip } from "../components/ui/tooltip";
import { AppIcon } from "../features/shared/AppIcon";
import { cn } from "../lib/utils";
import { useTheme } from "../theme";
import { demoModel } from "./model";
import type { DemoScene, DemoTheme } from "./runtime";

const sceneOptions: Array<{ value: DemoScene; label: string }> = [
  { value: "operational", label: "正常" },
  { value: "attention", label: "告警" },
  { value: "empty", label: "空态" },
  { value: "progressive-loading", label: "渐进加载" },
  { value: "network-failure", label: "网络失败" },
];

function useDemoSnapshot() {
  return useSyncExternalStore(
    (listener) => demoModel.subscribe(listener),
    () => demoModel.snapshot,
    () => demoModel.snapshot,
  );
}

function InspectorControls({ onClose }: { onClose?: () => void }) {
  const snapshot = useDemoSnapshot();
  const { pathname, search } = useLocation();
  const navigate = useNavigate();
  const { themeMode, setThemeMode } = useTheme();

  const updateShareState = (key: string, value: string) => {
    const params = new URLSearchParams(search);
    params.set(key, value);
    navigate({ pathname, search: `?${params.toString()}` }, { replace: true });
  };

  const setScene = (scene: DemoScene) => {
    demoModel.setScene(scene);
    updateShareState("demoScene", scene);
  };

  const setTheme = (theme: DemoTheme) => {
    setThemeMode(theme);
    updateShareState("demoTheme", theme);
  };

  const copyLink = async () => {
    const url = window.location.href;
    try {
      await navigator.clipboard.writeText(url);
      demoModel.record("复制 demo 分享链接");
    } catch {
      demoModel.record("生成 demo 分享链接");
    }
  };

  return (
    <div className="space-y-4" data-testid="demo-inspector-controls">
      <fieldset className="space-y-2">
        <legend className="text-xs font-semibold text-base-content/70">场景</legend>
        <div className="grid grid-cols-2 gap-2">
          {sceneOptions.map((option) => (
            <button
              key={option.value}
              type="button"
              aria-pressed={snapshot.scene === option.value}
              onClick={() => setScene(option.value)}
              className={cn(
                "h-9 rounded-md border px-3 text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary",
                snapshot.scene === option.value
                  ? "border-primary bg-primary/10 text-primary"
                  : "border-base-300 bg-base-100 text-base-content hover:bg-base-200",
              )}
            >
              {option.label}
            </button>
          ))}
        </div>
      </fieldset>
      <fieldset className="space-y-2">
        <legend className="text-xs font-semibold text-base-content/70">主题</legend>
        <div className="grid grid-cols-2 gap-2">
          {(["light", "dark"] as const).map((theme) => (
            <button
              key={theme}
              type="button"
              aria-pressed={themeMode === theme}
              onClick={() => setTheme(theme)}
              className={cn(
                "h-9 rounded-md border px-3 text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary",
                themeMode === theme
                  ? "border-primary bg-primary/10 text-primary"
                  : "border-base-300 bg-base-100 text-base-content hover:bg-base-200",
              )}
            >
              {theme === "light" ? "浅色" : "深色"}
            </button>
          ))}
        </div>
      </fieldset>
      <div className="flex items-center gap-1 border-t border-base-300 pt-3">
        <Tooltip content="注入模拟实时事件">
          <Button
            size="icon"
            variant="ghost"
            aria-label="注入模拟实时事件"
            onClick={() => demoModel.injectLiveEvent()}
          >
            <AppIcon name="lightning-bolt" className="h-4 w-4" aria-hidden />
          </Button>
        </Tooltip>
        <Tooltip content="重置模拟数据">
          <Button
            size="icon"
            variant="ghost"
            aria-label="重置模拟数据"
            onClick={() => demoModel.reset()}
          >
            <AppIcon name="refresh" className="h-4 w-4" aria-hidden />
          </Button>
        </Tooltip>
        <Tooltip content="复制分享链接">
          <Button
            size="icon"
            variant="ghost"
            aria-label="复制分享链接"
            onClick={() => void copyLink()}
          >
            <AppIcon name="content-copy" className="h-4 w-4" aria-hidden />
          </Button>
        </Tooltip>
        {onClose ? (
          <Button size="sm" variant="secondary" className="ml-auto" onClick={onClose}>
            完成
          </Button>
        ) : null}
      </div>
      {snapshot.actions.length > 0 ? (
        <div
          className="space-y-1 border-t border-base-300 pt-3 text-xs text-base-content/70"
          aria-live="polite"
        >
          {snapshot.actions.slice(0, 3).map((action) => (
            <p key={action.id}>{action.label}</p>
          ))}
        </div>
      ) : null}
    </div>
  );
}

export function DemoInspector({ defaultOpen = false }: { defaultOpen?: boolean }) {
  const [mobileOpen, setMobileOpen] = useState(false);
  const [desktopOpen, setDesktopOpen] = useState(defaultOpen);

  return (
    <>
      <aside
        className="fixed bottom-4 right-4 z-[70] hidden w-[min(22rem,calc(100vw-2rem))] sm:block"
        aria-label="Demo Inspector"
        data-testid="demo-inspector"
      >
        <details
          className="overflow-hidden rounded-lg border border-base-300 bg-base-100 shadow-sm"
          open={desktopOpen}
          onToggle={(event) => setDesktopOpen(event.currentTarget.open)}
        >
          <summary
            className="flex cursor-pointer list-none items-center justify-between gap-3 px-4 py-3 text-sm font-semibold text-base-content marker:hidden"
            data-testid="demo-inspector-summary"
          >
            <span>Demo Inspector</span>
            <span className="rounded-full bg-secondary/15 px-2 py-0.5 text-xs text-secondary">
              模拟数据
            </span>
          </summary>
          <div className="border-t border-base-300 p-4">
            <InspectorControls />
          </div>
        </details>
      </aside>
      <aside className="fixed bottom-4 right-4 z-[70] sm:hidden" aria-label="打开 Demo Inspector">
        <Tooltip content="Demo Inspector">
          <Button
            size="icon"
            variant="default"
            aria-label="打开 Demo Inspector"
            onClick={() => setMobileOpen(true)}
          >
            <AppIcon name="cog-outline" className="h-5 w-5" aria-hidden />
          </Button>
        </Tooltip>
      </aside>
      <Dialog open={mobileOpen} onOpenChange={setMobileOpen}>
        <DialogContent className="left-0 top-auto bottom-0 w-full max-w-none -translate-x-0 -translate-y-0 rounded-b-none rounded-t-lg border-base-300 bg-base-100 p-5">
          <DialogHeader>
            <DialogTitle className="text-lg">Demo Inspector</DialogTitle>
            <DialogDescription>所有结果均来自本地模拟数据。</DialogDescription>
          </DialogHeader>
          <div className="mt-5">
            <InspectorControls onClose={() => setMobileOpen(false)} />
          </div>
        </DialogContent>
      </Dialog>
    </>
  );
}
