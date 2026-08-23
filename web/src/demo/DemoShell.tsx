import { type ReactNode, useEffect, useMemo, useSyncExternalStore } from "react";
import { useLocation } from "react-router-dom";
import { useTheme } from "../theme";
import { demoModel } from "./model";
import {
  demoSearchParamsFromLocation,
  isEmbeddedDemoViewport,
  sceneFromLocation,
  themeFromLocation,
  viewportFromLocation,
} from "./runtime";

export function DemoShell({ children }: { children: ReactNode }) {
  const location = useLocation();
  const { setThemeMode } = useTheme();
  const snapshot = useSyncExternalStore(
    (listener) => demoModel.subscribe(listener),
    () => demoModel.snapshot,
    () => demoModel.snapshot,
  );

  useEffect(() => {
    void location.key;
    demoModel.setScene(sceneFromLocation());
    setThemeMode(themeFromLocation());
  }, [location.key, setThemeMode]);

  const viewport = viewportFromLocation();
  const embedded = isEmbeddedDemoViewport();
  const mobileViewport = viewport === "mobile390" || viewport === "mobile393";
  const mobileViewportWidth = viewport === "mobile393" ? 393 : 390;
  const mobileViewportHeight = viewport === "mobile393" ? 852 : 844;
  const embeddedSrc = useMemo(() => {
    if (typeof window === "undefined" || !mobileViewport || embedded) return null;

    const search = demoSearchParamsFromLocation();
    search.delete("demoViewport");
    search.set("demoEmbed", "1");
    const hash = `${location.pathname}${search.size > 0 ? `?${search.toString()}` : ""}`;
    return `${window.location.pathname}#${hash}`;
  }, [embedded, location.pathname, mobileViewport]);

  if (mobileViewport && !embedded && embeddedSrc) {
    return (
      <div className="min-h-screen bg-base-200 px-4 py-6 text-base-content sm:px-6">
        <div className="mx-auto flex w-full max-w-[30rem] flex-col gap-3">
          <p className="text-xs font-semibold uppercase tracking-[0.16em] text-base-content/55">
            移动视口 · {mobileViewportWidth}px
          </p>
          <div className="overflow-hidden rounded-[2rem] border border-base-300/80 bg-base-100 shadow-2xl shadow-base-content/10">
            <iframe
              title="移动视口"
              src={embeddedSrc}
              className="block max-w-full border-0 bg-base-100"
              style={{ height: mobileViewportHeight, width: mobileViewportWidth }}
            />
          </div>
        </div>
      </div>
    );
  }

  return <div key={snapshot.scene}>{children}</div>;
}
