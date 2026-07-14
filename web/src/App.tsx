import { Navigate, Route, Routes } from "react-router-dom";
import { AppLayout } from "./features/app-shell/AppLayout";
import AccountPoolLayout from "./pages/account-pool/AccountPoolLayout";
import GroupsPage from "./pages/account-pool/Groups";
import MaintenanceRecordsPage from "./pages/account-pool/MaintenanceRecords";
import UpstreamAccountCreatePage from "./pages/account-pool/UpstreamAccountCreate";
import UpstreamAccountsPage from "./pages/account-pool/UpstreamAccounts";
import DashboardPage from "./pages/Dashboard";
import LivePage from "./pages/Live";
import RecordsPage from "./pages/Records";
import SettingsPage from "./pages/Settings";
import StatsPage from "./pages/Stats";
import SystemLayout from "./pages/system/SystemLayout";
import SystemProxyPage from "./pages/system/SystemProxyPage";
import SystemSettingsPage from "./pages/system/SystemSettingsPage";
import SystemStatusPage from "./pages/system/SystemStatusPage";
import SystemTasksPage from "./pages/system/SystemTasksPage";

function App() {
  return (
    <Routes>
      <Route path="/" element={<AppLayout />}>
        <Route index element={<Navigate to="/dashboard" replace />} />
        <Route path="dashboard" element={<DashboardPage />} />
        <Route path="dashboard/invocations/:invokeId" element={<DashboardPage />} />
        <Route path="stats" element={<StatsPage />} />
        <Route path="live" element={<LivePage />} />
        <Route path="records" element={<RecordsPage />} />
        <Route path="account-pool" element={<AccountPoolLayout />}>
          <Route index element={<Navigate to="/account-pool/upstream-accounts" replace />} />
          <Route path="upstream-accounts" element={<UpstreamAccountsPage />} />
          <Route path="upstream-accounts/new" element={<UpstreamAccountCreatePage />} />
          <Route path="maintenance-records" element={<MaintenanceRecordsPage />} />
          <Route path="groups" element={<GroupsPage />} />
        </Route>
        <Route path="system" element={<SystemLayout />}>
          <Route index element={<Navigate to="/system/status" replace />} />
          <Route path="status" element={<SystemStatusPage />} />
          <Route path="tasks" element={<SystemTasksPage />} />
          <Route path="settings" element={<SystemSettingsPage />} />
          <Route path="proxy" element={<SystemProxyPage />} />
        </Route>
        <Route path="settings" element={<Navigate to="/system/settings" replace />} />
        <Route path="settings/legacy" element={<SettingsPage mode="all" />} />
        <Route path="*" element={<Navigate to="/dashboard" replace />} />
      </Route>
    </Routes>
  );
}

export default App;
