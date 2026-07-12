import { type ComponentType, Fragment, StrictMode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { HashRouter } from "react-router-dom";
import App from "./App.tsx";
import "./index.css";
import { SystemNotificationProvider } from "./components/ui/system-notifications";
import { DemoBootstrapFailure } from "./demo/DemoBootstrapFailure";
import { initializeDemoRuntime, isDemoRuntime } from "./demo/runtime";
import { I18nProvider } from "./i18n";
import { ThemeProvider } from "./theme";

const ROOT_KEY = Symbol.for("codex-vibe-monitor.react-root");
const rootElement = document.getElementById("root");

if (rootElement == null) {
  throw new Error("Missing application root element");
}

const applicationRoot = rootElement as HTMLElement & { [ROOT_KEY]?: Root };
const root = applicationRoot[ROOT_KEY] ?? createRoot(applicationRoot);
applicationRoot[ROOT_KEY] = root;

async function bootstrap() {
  try {
    const demo = isDemoRuntime();
    let RuntimeShell: ComponentType<{ children: React.ReactNode }> = Fragment;

    if (demo) {
      await initializeDemoRuntime();
      const module = await import("./demo/DemoShell");
      RuntimeShell = module.DemoShell;
    }

    root.render(
      <StrictMode>
        <ThemeProvider>
          <I18nProvider>
            <SystemNotificationProvider>
              <HashRouter>
                <RuntimeShell>
                  <App />
                </RuntimeShell>
              </HashRouter>
            </SystemNotificationProvider>
          </I18nProvider>
        </ThemeProvider>
      </StrictMode>,
    );
  } catch (error) {
    console.error("Unable to initialize application runtime.", error);
    root.render(<DemoBootstrapFailure />);
  }
}

void bootstrap();
