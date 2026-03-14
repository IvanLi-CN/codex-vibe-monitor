import { Navigate, Route, Routes } from 'react-router-dom'
import { AppLayout } from './components/AppLayout'
import DashboardPage from './pages/Dashboard'
import LivePage from './pages/Live'
import RecordsPage from './pages/Records'
import SettingsPage from './pages/Settings'
import StatsPage from './pages/Stats'
import AccountPoolLayout from './pages/account-pool/AccountPoolLayout'
import UpstreamAccountsPage from './pages/account-pool/UpstreamAccounts'
import UpstreamAccountCreatePage from './pages/account-pool/UpstreamAccountCreate'
import TagsPage from './pages/account-pool/Tags'

function App() {
  return (
    <Routes>
      <Route path="/" element={<AppLayout />}>
        <Route index element={<Navigate to="/dashboard" replace />} />
        <Route path="dashboard" element={<DashboardPage />} />
        <Route path="stats" element={<StatsPage />} />
        <Route path="live" element={<LivePage />} />
        <Route path="records" element={<RecordsPage />} />
        <Route path="account-pool" element={<AccountPoolLayout />}>
          <Route index element={<Navigate to="/account-pool/upstream-accounts" replace />} />
          <Route path="upstream-accounts" element={<UpstreamAccountsPage />} />
          <Route path="upstream-accounts/new" element={<UpstreamAccountCreatePage />} />
          <Route path="tags" element={<TagsPage />} />
        </Route>
        <Route path="settings" element={<SettingsPage />} />
        <Route path="*" element={<Navigate to="/dashboard" replace />} />
      </Route>
    </Routes>
  )
}

export default App
