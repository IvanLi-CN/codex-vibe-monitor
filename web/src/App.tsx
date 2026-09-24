import { Component, type ErrorInfo, lazy, type ReactNode, Suspense } from "react";
import { Navigate, Route, Routes, useLocation } from "react-router-dom";
import { Spinner } from "./components/ui/spinner";
import { AppLayout } from "./features/app-shell/AppLayout";

const AccountPoolLayout = lazy(() => import("./pages/account-pool/AccountPoolLayout"));
const GroupsPage = lazy(() => import("./pages/account-pool/Groups"));
const MaintenanceRecordsPage = lazy(() => import("./pages/account-pool/MaintenanceRecords"));
const UpstreamAccountCreatePage = lazy(() => import("./pages/account-pool/UpstreamAccountCreate"));
const UpstreamAccountsPage = lazy(() => import("./pages/account-pool/UpstreamAccounts"));
const DashboardPage = lazy(() => import("./pages/Dashboard"));
const LivePage = lazy(() => import("./pages/Live"));
const RecordsPage = lazy(() => import("./pages/Records"));
const SettingsPage = lazy(() => import("./pages/Settings"));
const StatsPage = lazy(() => import("./pages/Stats"));
const SystemLayout = lazy(() => import("./pages/system/SystemLayout"));
const SystemProxyPage = lazy(() => import("./pages/system/SystemProxyPage"));
const SystemSettingsPage = lazy(() => import("./pages/system/SystemSettingsPage"));
const SystemStatusPage = lazy(() => import("./pages/system/SystemStatusPage"));
const SystemTasksPage = lazy(() => import("./pages/system/SystemTasksPage"));

function RouteLoadingFallback() {
  return (
    <div className="flex min-h-screen items-center justify-center" role="status">
      <Spinner aria-label="Loading page" />
    </div>
  );
}

type AppErrorBoundaryProps = {
  children: ReactNode;
};

type AppErrorBoundaryState = {
  hasError: boolean;
};

export class AppErrorBoundary extends Component<AppErrorBoundaryProps, AppErrorBoundaryState> {
  state: AppErrorBoundaryState = { hasError: false };

  static getDerivedStateFromError(): AppErrorBoundaryState {
    return { hasError: true };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("Failed to load the application route", error, info);
  }

  handleRetry = () => {
    window.location.reload();
  };

  render() {
    if (this.state.hasError) {
      return (
        <div className="flex min-h-screen items-center justify-center p-6" role="alert">
          <div className="surface-panel flex max-w-md flex-col gap-3 p-6 text-center">
            <h1 className="text-base font-semibold text-base-content">页面加载失败</h1>
            <p className="text-sm text-base-content/70">请重新加载后再试。</p>
            <button
              className="btn btn-primary self-center"
              type="button"
              onClick={this.handleRetry}
            >
              重新加载
            </button>
          </div>
        </div>
      );
    }

    return this.props.children;
  }
}

function LegacyAccountCreateRedirect() {
  const location = useLocation();
  const target =
    new URLSearchParams(location.search).get("mode") === "apiKey"
      ? "/account-pool/transits/new"
      : "/account-pool/pool/new";
  return <Navigate to={`${target}${location.search}`} replace />;
}

function App() {
  return (
    <AppErrorBoundary>
      <Suspense fallback={<RouteLoadingFallback />}>
        <Routes>
          <Route path="/" element={<AppLayout />}>
            <Route index element={<Navigate to="/dashboard" replace />} />
            <Route path="dashboard" element={<DashboardPage />} />
            <Route path="dashboard/invocations/:invokeId" element={<DashboardPage />} />
            <Route path="stats" element={<StatsPage />} />
            <Route path="live" element={<LivePage />} />
            <Route path="records" element={<RecordsPage />} />
            <Route path="account-pool" element={<AccountPoolLayout />}>
              <Route index element={<Navigate to="/account-pool/pool" replace />} />
              <Route path="transits" element={<UpstreamAccountsPage />} />
              <Route path="transits/new" element={<UpstreamAccountCreatePage />} />
              <Route path="pool" element={<UpstreamAccountsPage />} />
              <Route path="pool/new" element={<UpstreamAccountCreatePage />} />
              <Route
                path="upstream-accounts"
                element={<Navigate to="/account-pool/pool" replace />}
              />
              <Route path="upstream-accounts/new" element={<LegacyAccountCreateRedirect />} />
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
      </Suspense>
    </AppErrorBoundary>
  );
}

export default App;
