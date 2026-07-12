import { type ReactNode, useEffect, useSyncExternalStore } from "react";
import { useLocation } from "react-router-dom";
import { useTheme } from "../theme";
import { DemoInspector } from "./DemoInspector";
import { demoModel } from "./model";
import { sceneFromLocation, themeFromLocation } from "./runtime";

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

  return (
    <>
      <div key={snapshot.scene}>{children}</div>
      <DemoInspector />
    </>
  );
}
