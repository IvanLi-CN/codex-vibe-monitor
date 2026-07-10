import { Fragment, StrictMode, type ComponentType } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { HashRouter } from 'react-router-dom'
import App from './App.tsx'
import './index.css'
import { I18nProvider } from './i18n'
import { SystemNotificationProvider } from './components/ui/system-notifications'
import { ThemeProvider } from './theme'
import { initializeDemoRuntime, isDemoRuntime } from './demo/runtime'
import { DemoBootstrapFailure } from './demo/DemoBootstrapFailure'

const ROOT_KEY = Symbol.for('codex-vibe-monitor.react-root')
const rootElement = document.getElementById('root')! as HTMLElement & { [ROOT_KEY]?: Root }
const root = rootElement[ROOT_KEY] ?? createRoot(rootElement)
rootElement[ROOT_KEY] = root

async function bootstrap() {
  try {
    const demo = isDemoRuntime()
    let RuntimeShell: ComponentType<{ children: React.ReactNode }> = Fragment

    if (demo) {
      await initializeDemoRuntime()
      const module = await import('./demo/DemoShell')
      RuntimeShell = module.DemoShell
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
    )
  } catch (error) {
    console.error('Unable to initialize application runtime.', error)
    root.render(<DemoBootstrapFailure />)
  }
}

void bootstrap()
