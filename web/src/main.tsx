import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { HashRouter } from 'react-router-dom'
import App from './App.tsx'
import './index.css'
import { I18nProvider } from './i18n'
import { SystemNotificationProvider } from './components/ui/system-notifications'
import { ThemeProvider } from './theme'

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <ThemeProvider>
      <I18nProvider>
        <SystemNotificationProvider>
          <HashRouter>
            <App />
          </HashRouter>
        </SystemNotificationProvider>
      </I18nProvider>
    </ThemeProvider>
  </StrictMode>,
)
