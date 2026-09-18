import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { NavLink, useLocation } from "react-router-dom";
import { Chip } from "../../components/ui/chip";
import { SegmentedControl } from "../../components/ui/segmented-control";
import { segmentedControlItemVariants } from "../../components/ui/segmented-control.variants";
import type { PwaRuntimeState } from "../../hooks/usePwaRuntime";
import { type Locale, supportedLocales, useTranslation } from "../../i18n";
import { type SseStatus, subscribeToSseActivity } from "../../lib/sse";
import { frontendVersion, normalizeVersion } from "../../lib/version";
import { useTheme } from "../../theme";
import { AppIcon } from "../shared/AppIcon";
import { HeaderBrandMark, type HeaderBrandMarkState } from "./HeaderBrandMark";
import { LanguageFlag } from "./LanguageFlag";
import {
  desktopNavItems,
  matchesNavigationPath,
  mobileNavigationGroups,
  resolveAppNavigation,
} from "./navigation";
import { PwaInstallControl } from "./PwaInstallControl";
import { PwaInstallTrigger } from "./PwaInstallTrigger";
import { UpdateAvailableBanner } from "./UpdateAvailableBanner";

const repositoryUrl = "https://github.com/IvanLi-CN/codex-vibe-monitor";
const LOCALE_FLAG: Record<Locale, string> = { zh: "🇨🇳", en: "🇺🇸" };
const HEADER_BRAND_ACTIVITY_HOLD_MS = 3200;

type Translate = (key: string, values?: Record<string, string | number>) => string;
type InstallPromptMode = Extract<PwaRuntimeState["installMode"], "prompt" | "manual-ios">;

export function usePwaInstallDialog(pwaRuntime: PwaRuntimeState) {
  const mode: InstallPromptMode | null =
    pwaRuntime.installMode === "prompt" || pwaRuntime.installMode === "manual-ios"
      ? pwaRuntime.installMode
      : null;
  const [isOpen, setIsOpen] = useState(false);
  const suppressDeferRef = useRef(false);
  useEffect(() => {
    if (pwaRuntime.shouldAutoOpenInstallDialog && mode) setIsOpen(true);
  }, [mode, pwaRuntime.shouldAutoOpenInstallDialog]);
  const onOpenChange = useCallback(
    (nextOpen: boolean) => {
      if (nextOpen) return setIsOpen(true);
      if (mode && !suppressDeferRef.current) pwaRuntime.deferInstallPrompt();
      suppressDeferRef.current = false;
      setIsOpen(false);
    },
    [mode, pwaRuntime.deferInstallPrompt],
  );
  const open = useCallback(() => setIsOpen(true), []);
  const promptInstall = useCallback(async () => {
    suppressDeferRef.current = true;
    try {
      await pwaRuntime.promptInstall();
    } finally {
      setIsOpen(false);
    }
  }, [pwaRuntime.promptInstall]);
  return { isOpen, mode, onOpenChange, open, promptInstall };
}

export function AppInstallDialog({
  pwaRuntime,
  state,
}: {
  pwaRuntime: PwaRuntimeState;
  state: ReturnType<typeof usePwaInstallDialog>;
}) {
  const { t } = useTranslation();
  if (!state.mode) return null;
  return (
    <PwaInstallControl
      open={state.isOpen}
      onOpenChange={state.onOpenChange}
      mode={state.mode}
      shellReady={pwaRuntime.shellReady}
      isOffline={pwaRuntime.isOffline}
      onPromptInstall={state.promptInstall}
      canPromptInstall={pwaRuntime.installPromptAvailable}
      labels={{
        promptButton: t("app.pwa.install.promptButton"),
        laterButton: t("app.pwa.install.laterButton"),
        manualButton: t("app.pwa.install.manualButton"),
        installedButton: t("app.pwa.install.installedButton"),
        switcherAria: t("app.pwa.install.switcherAria"),
        closeButton: t("app.pwa.install.close"),
        closeAria: t("app.pwa.install.closeAria"),
        shellReady: t("app.pwa.install.shellReady"),
        shellPending: t("app.pwa.install.shellPending"),
        offlineChip: t("app.pwa.install.offlineChip"),
        promptTitle: t("app.pwa.install.promptTitle"),
        promptDescription: t("app.pwa.install.promptDescription"),
        promptHint: t("app.pwa.install.promptHint"),
        manualTitle: t("app.pwa.install.manualTitle"),
        manualDescription: t("app.pwa.install.manualDescription"),
        manualStepOpenShare: t("app.pwa.install.manualStepOpenShare"),
        manualStepAdd: t("app.pwa.install.manualStepAdd"),
        manualStepConfirm: t("app.pwa.install.manualStepConfirm"),
        installedTitle: t("app.pwa.install.installedTitle"),
        installedDescription: t("app.pwa.install.installedDescription"),
        installedHint: t("app.pwa.install.installedHint"),
      }}
    />
  );
}

export function useHeaderBrandMarkState(phase: SseStatus["phase"]): HeaderBrandMarkState {
  const [hasRecentActivity, setHasRecentActivity] = useState(false);
  const activityTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    const clearActivityWindow = () => {
      if (!activityTimeoutRef.current) return;
      clearTimeout(activityTimeoutRef.current);
      activityTimeoutRef.current = null;
    };
    const unsubscribe = subscribeToSseActivity(() => {
      if (phase !== "connected") return;
      setHasRecentActivity(true);
      clearActivityWindow();
      activityTimeoutRef.current = setTimeout(() => {
        activityTimeoutRef.current = null;
        setHasRecentActivity(false);
      }, HEADER_BRAND_ACTIVITY_HOLD_MS);
    });
    return () => {
      clearActivityWindow();
      unsubscribe();
    };
  }, [phase]);

  useEffect(() => {
    if (phase === "connected") return;
    if (activityTimeoutRef.current) clearTimeout(activityTimeoutRef.current);
    activityTimeoutRef.current = null;
    setHasRecentActivity(false);
  }, [phase]);

  if (phase === "disabled") return "disabled";
  if (phase === "connecting" || phase === "reconnecting") return "reconnecting";
  return hasRecentActivity ? "active" : "idle";
}

interface AppHeaderProps {
  brandMarkState: HeaderBrandMarkState;
  installDialogMode: InstallPromptMode | null;
  onOpenInstallDialog: () => void;
}

export function AppHeader({
  brandMarkState,
  installDialogMode,
  onOpenInstallDialog,
}: AppHeaderProps) {
  const { t } = useTranslation();
  const location = useLocation();
  const [mobileNavOpen, setMobileNavOpen] = useState(false);
  const resolvedNavigation = useMemo(
    () => resolveAppNavigation(location.pathname),
    [location.pathname],
  );
  const mobileContextLabel = t(
    resolvedNavigation.nestedItem?.labelKey ?? resolvedNavigation.topLevelItem.labelKey,
  );
  const mobileContextEyebrow = resolvedNavigation.nestedItem
    ? t(resolvedNavigation.topLevelItem.labelKey)
    : null;

  useMobileNavigationBehavior({ mobileNavOpen, setMobileNavOpen });

  return (
    <>
      <header className="sticky top-0 z-50 border-b border-base-300/75 bg-base-100/80 backdrop-blur-md">
        <div
          className="app-shell-boundary flex items-center gap-2 px-3 py-2 sm:px-4"
          data-testid="app-header-inner"
        >
          <AppHeaderIdentity
            brandMarkState={brandMarkState}
            mobileContextEyebrow={mobileContextEyebrow}
            mobileContextLabel={mobileContextLabel}
            mobileNavOpen={mobileNavOpen}
            onOpenMobileNav={() => setMobileNavOpen(true)}
            t={t}
          />
          <AppHeaderControls
            installDialogMode={installDialogMode}
            onOpenInstallDialog={onOpenInstallDialog}
            pathname={location.pathname}
          />
        </div>
      </header>
      {mobileNavOpen ? (
        <AppMobileNavigation
          brandMarkState={brandMarkState}
          mobileContextLabel={mobileContextLabel}
          onClose={() => setMobileNavOpen(false)}
          pathname={location.pathname}
          t={t}
        />
      ) : null}
    </>
  );
}

function useMobileNavigationBehavior({
  mobileNavOpen,
  setMobileNavOpen,
}: {
  mobileNavOpen: boolean;
  setMobileNavOpen: (open: boolean) => void;
}) {
  useEffect(() => {
    setMobileNavOpen(false);
  }, [setMobileNavOpen]);
  useEffect(() => {
    if (!mobileNavOpen || typeof document === "undefined") return undefined;
    const previousOverflow = document.body.style.overflow;
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setMobileNavOpen(false);
    };
    document.body.style.overflow = "hidden";
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.body.style.overflow = previousOverflow;
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [mobileNavOpen, setMobileNavOpen]);
}

function AppHeaderIdentity({
  brandMarkState,
  mobileContextEyebrow,
  mobileContextLabel,
  mobileNavOpen,
  onOpenMobileNav,
  t,
}: {
  brandMarkState: HeaderBrandMarkState;
  mobileContextEyebrow: string | null;
  mobileContextLabel: string;
  mobileNavOpen: boolean;
  onOpenMobileNav: () => void;
  t: Translate;
}) {
  return (
    <div className="flex min-w-0 flex-1 items-center gap-2.5 sm:gap-3">
      <button
        type="button"
        className="control-pill desktop:!hidden"
        onClick={onOpenMobileNav}
        aria-label={t("app.nav.openMenu")}
        aria-expanded={mobileNavOpen}
        aria-controls="app-mobile-navigation"
      >
        <AppIcon name="menu" className="h-[18px] w-[18px] text-primary" aria-hidden />
        <span className="sr-only">{t("app.nav.openMenu")}</span>
      </button>
      <HeaderBrandMark
        alt={t("app.logoAlt")}
        state={brandMarkState}
        className="h-9 w-9 desktop:h-10 desktop:w-10"
        markClassName="h-9 w-9 desktop:h-10 desktop:w-10"
        data-testid="app-header-logo-mark"
      />
      <div className="min-w-0">
        <span className="hidden truncate text-lg font-semibold tracking-tight desktop:block desktop:text-xl">
          {t("app.brand")}
        </span>
        <div className="desktop:hidden">
          {mobileContextEyebrow ? (
            <p className="truncate text-[10px] font-semibold uppercase tracking-[0.18em] text-primary/72">
              {mobileContextEyebrow}
            </p>
          ) : null}
          <p className="truncate text-sm font-semibold tracking-tight">{mobileContextLabel}</p>
        </div>
      </div>
    </div>
  );
}

function AppHeaderControls({
  installDialogMode,
  onOpenInstallDialog,
  pathname,
}: {
  installDialogMode: InstallPromptMode | null;
  onOpenInstallDialog: () => void;
  pathname: string;
}) {
  const { t } = useTranslation();
  const { themeMode, toggleTheme } = useTheme();
  const isDarkTheme = themeMode === "dark";
  const themeLabel = t(isDarkTheme ? "app.theme.currentDark" : "app.theme.currentLight");
  const themeSwitcherLabel = t(isDarkTheme ? "app.theme.switchToLight" : "app.theme.switchToDark");
  return (
    <nav className="flex shrink-0 items-center gap-2 sm:gap-3">
      {installDialogMode ? (
        <AppInstallTrigger mode={installDialogMode} onOpen={onOpenInstallDialog} />
      ) : null}
      <AppDesktopNavigation pathname={pathname} t={t} />
      <button
        type="button"
        className="control-pill"
        onClick={toggleTheme}
        aria-label={t("app.theme.switcherAria")}
        title={themeSwitcherLabel}
      >
        <AppIcon
          name={isDarkTheme ? "weather-night" : "white-balance-sunny"}
          className="h-[18px] w-[18px] text-primary"
          aria-hidden
        />
        <span className="hidden md:inline">{themeLabel}</span>
      </button>
      <MobileLanguageToggle />
      <DesktopLanguageMenu />
    </nav>
  );
}

function AppInstallTrigger({ mode, onOpen }: { mode: InstallPromptMode; onOpen: () => void }) {
  const { t } = useTranslation();
  const trigger = (
    <PwaInstallTrigger
      mode={mode}
      label={t("app.pwa.install.promptButton")}
      ariaLabel={t("app.pwa.install.switcherAria")}
      onClick={onOpen}
    />
  );
  return (
    <>
      <div className="hidden desktop:block">{trigger}</div>
      <div className="desktop:hidden">
        <PwaInstallTrigger
          mode={mode}
          label={t("app.pwa.install.promptButton")}
          ariaLabel={t("app.pwa.install.switcherAria")}
          compact
          onClick={onOpen}
        />
      </div>
    </>
  );
}

function AppDesktopNavigation({ pathname, t }: { pathname: string; t: Translate }) {
  return (
    <div className="hidden overflow-x-auto no-scrollbar desktop:block">
      <SegmentedControl size="nav" className="min-w-max" aria-label={t("app.brand")}>
        {desktopNavItems.map((item) => (
          <NavLink
            key={item.to}
            to={item.to}
            aria-current={matchesNavigationPath(pathname, item) ? "page" : undefined}
            className={segmentedControlItemVariants({
              size: "nav",
              active: matchesNavigationPath(pathname, item),
            })}
          >
            {t(item.labelKey)}
          </NavLink>
        ))}
      </SegmentedControl>
    </div>
  );
}

function MobileLanguageToggle() {
  const { t, locale, setLocale } = useTranslation();
  return (
    <div className="desktop:hidden">
      <button
        type="button"
        className="control-pill"
        aria-label={t("app.language.switcherAria")}
        onClick={() => setLocale(locale === "zh" ? "en" : "zh")}
        data-testid="mobile-language-toggle"
      >
        <LanguageFlag locale={locale} />
      </button>
    </div>
  );
}

function DesktopLanguageMenu() {
  const { t, locale, setLocale } = useTranslation();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement | null>(null);
  const choices = useMemo(
    () =>
      supportedLocales.map((code) => ({
        code,
        flag: LOCALE_FLAG[code],
        label: t(code === "zh" ? "app.language.option.zh" : "app.language.option.en"),
      })),
    [t],
  );
  const close = useCallback(() => setOpen(false), []);
  useEffect(() => {
    if (!open) return undefined;
    const closeOutside = (event: MouseEvent) => {
      if (!ref.current?.contains(event.target as Node)) close();
    };
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") close();
    };
    document.addEventListener("mousedown", closeOutside);
    document.addEventListener("keydown", closeOnEscape);
    return () => {
      document.removeEventListener("mousedown", closeOutside);
      document.removeEventListener("keydown", closeOnEscape);
    };
  }, [close, open]);
  const activeChoice = choices.find((choice) => choice.code === locale) ?? choices[0];
  return (
    <div ref={ref} className="relative hidden desktop:block">
      <button
        type="button"
        className="control-pill min-w-0 justify-between sm:min-w-[6.75rem]"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label={t("app.language.switcherAria")}
        onClick={() => setOpen((value) => !value)}
      >
        <AppIcon name="earth" className="h-[18px] w-[18px] text-base-content/75" aria-hidden />
        <span className="hidden sm:inline">{activeChoice?.label}</span>
        <AppIcon name="chevron-down" className="h-4 w-4 text-base-content/60" aria-hidden />
      </button>
      <ul
        className={`absolute right-0 top-[calc(100%+0.4rem)] z-50 mt-2 min-w-[10.5rem] rounded-xl border border-base-300 bg-base-100/95 p-2 shadow-lg backdrop-blur ${open ? "block" : "hidden"}`}
        aria-label={t("app.language.switcherAria")}
      >
        {choices.map((choice) => (
          <li key={choice.code} role="presentation">
            <button
              type="button"
              className={`flex items-center gap-2 rounded-md px-2 py-1 ${choice.code === locale ? "bg-primary/15 font-medium text-primary" : "hover:bg-base-200"}`}
              onClick={() => {
                if (choice.code !== locale) setLocale(choice.code);
                close();
              }}
              role="option"
              aria-selected={choice.code === locale}
            >
              <span aria-hidden>{choice.flag}</span>
              <span>{choice.label}</span>
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}

function AppMobileNavigation({
  brandMarkState,
  mobileContextLabel,
  onClose,
  pathname,
  t,
}: {
  brandMarkState: HeaderBrandMarkState;
  mobileContextLabel: string;
  onClose: () => void;
  pathname: string;
  t: Translate;
}) {
  return (
    <div className="fixed inset-0 z-[85] desktop:hidden">
      <button
        type="button"
        aria-label={t("app.nav.closeMenu")}
        className="absolute inset-0 bg-neutral/56 backdrop-blur-sm"
        onClick={onClose}
      />
      <aside
        id="app-mobile-navigation"
        className="absolute inset-y-0 left-0 flex w-[min(22rem,calc(100vw-1rem))] max-w-full flex-col border-r border-base-300/75 bg-base-100/96 px-4 pb-[max(env(safe-area-inset-bottom),1rem)] pt-[max(env(safe-area-inset-top),1rem)] shadow-[0_24px_72px_rgba(15,23,42,0.2)] backdrop-blur-xl"
      >
        <div className="flex items-start justify-between gap-3 border-b border-base-300/70 pb-4">
          <div className="flex min-w-0 items-center gap-3">
            <HeaderBrandMark
              alt={t("app.logoAlt")}
              state={brandMarkState}
              className="h-9 w-9"
              markClassName="h-9 w-9"
            />
            <div className="min-w-0">
              <p className="truncate text-sm font-semibold tracking-tight">{t("app.brand")}</p>
              <p className="truncate text-xs text-base-content/62">{mobileContextLabel}</p>
            </div>
          </div>
          <button
            type="button"
            className="control-pill"
            onClick={onClose}
            aria-label={t("app.nav.closeMenu")}
          >
            <AppIcon name="close" className="h-[18px] w-[18px] text-base-content/78" aria-hidden />
            <span className="sr-only">{t("app.nav.closeMenu")}</span>
          </button>
        </div>
        <nav className="flex min-h-0 flex-1 flex-col gap-5 overflow-y-auto py-4 pr-1">
          <MobileStandaloneNavigation pathname={pathname} t={t} />
          <MobileGroupedNavigation pathname={pathname} t={t} />
        </nav>
      </aside>
    </div>
  );
}

function MobileStandaloneNavigation({ pathname, t }: { pathname: string; t: Translate }) {
  return (
    <div className="flex flex-col gap-1.5">
      {mobileNavigationGroups
        .filter((group) => group.items.length === 0)
        .map((item) => (
          <NavLink
            key={item.to}
            to={item.to}
            className={[
              "flex items-center justify-between rounded-2xl border px-4 py-3 text-left text-sm font-medium transition-colors",
              matchesNavigationPath(pathname, item)
                ? "border-primary/45 bg-primary/12 text-primary"
                : "border-base-300/70 bg-base-100/72 text-base-content/78 hover:border-primary/30 hover:text-base-content",
            ].join(" ")}
          >
            <span>{t(item.labelKey)}</span>
            <AppIcon name="chevron-right" className="h-4 w-4" aria-hidden />
          </NavLink>
        ))}
    </div>
  );
}

function MobileGroupedNavigation({ pathname, t }: { pathname: string; t: Translate }) {
  return (
    <>
      {mobileNavigationGroups
        .filter((group) => group.items.length > 0)
        .map((group) => (
          <section key={group.to} className="space-y-2">
            <p className="px-1 text-[11px] font-semibold uppercase tracking-[0.18em] text-base-content/52">
              {t(group.labelKey)}
            </p>
            <div className="flex flex-col gap-1.5">
              {group.items.map((item) => (
                <NavLink
                  key={item.to}
                  to={item.to}
                  className={[
                    "rounded-2xl border px-4 py-3 text-sm font-medium transition-colors",
                    matchesNavigationPath(pathname, item)
                      ? "border-primary/45 bg-primary/12 text-primary"
                      : "border-base-300/70 bg-base-100/72 text-base-content/78 hover:border-primary/30 hover:text-base-content",
                  ].join(" ")}
                >
                  {t(item.labelKey)}
                </NavLink>
              ))}
            </div>
          </section>
        ))}
    </>
  );
}

export function AppPwaOfflineBanner({
  isOffline,
  shellReady,
  topClassName,
}: {
  isOffline: boolean;
  shellReady: boolean;
  topClassName: string;
}) {
  const { t } = useTranslation();
  if (!isOffline) return null;
  return (
    <div className={`fixed left-1/2 z-[62] w-full max-w-3xl -translate-x-1/2 px-4 ${topClassName}`}>
      <div
        className="flex w-full flex-col gap-3 rounded-xl border border-warning/45 bg-base-100/95 p-4 text-base-content shadow-lg backdrop-blur sm:flex-row sm:items-center"
        role="status"
        aria-live="assertive"
        data-testid="pwa-offline-banner"
      >
        <div className="flex min-w-0 flex-1 items-center gap-3">
          <AppIcon
            name="link-variant-off"
            className="h-6 w-6 flex-shrink-0 text-warning"
            aria-hidden
          />
          <div className="min-w-0 space-y-1">
            <div className="flex flex-wrap items-center gap-3">
              <span className="font-semibold">{t("app.pwa.offline.title")}</span>
              <Chip
                size="compact"
                tone={shellReady ? "success" : "warning"}
                className="px-2 text-xs font-medium"
              >
                {shellReady ? t("app.pwa.install.shellReady") : t("app.pwa.install.shellPending")}
              </Chip>
            </div>
            <p className="text-sm text-base-content/78">
              {shellReady
                ? t("app.pwa.offline.descriptionReady")
                : t("app.pwa.offline.descriptionPending")}
            </p>
          </div>
        </div>
      </div>
    </div>
  );
}

export function AppUpdateBanners({
  backendVersion,
  pwaRuntime,
  update,
}: {
  backendVersion: string | undefined;
  pwaRuntime: PwaRuntimeState;
  update: {
    visible: boolean;
    availableVersion: string | null;
    reload: () => void;
    dismiss: () => void;
  };
}) {
  const { t } = useTranslation();
  if (pwaRuntime.update.visible)
    return (
      <UpdateAvailableBanner
        currentVersion={pwaRuntime.update.currentVersion}
        availableVersion={pwaRuntime.update.availableVersion ?? pwaRuntime.update.currentVersion}
        onReload={pwaRuntime.applyUpdate}
        onDismiss={pwaRuntime.dismissUpdate}
        labels={{
          available: t("app.pwa.update.available"),
          refresh: t("app.pwa.update.refresh"),
          later: t("app.pwa.update.later"),
        }}
      />
    );
  if (!update.visible || !update.availableVersion) return null;
  return (
    <UpdateAvailableBanner
      currentVersion={backendVersion ?? t("app.update.current")}
      availableVersion={update.availableVersion}
      onReload={update.reload}
      onDismiss={update.dismiss}
      labels={{
        available: t("app.update.available"),
        refresh: t("app.update.refresh"),
        later: t("app.update.later"),
      }}
    />
  );
}

export function AppFooter({
  backendVersion,
  isBackendLoading,
}: {
  backendVersion: string | undefined;
  isBackendLoading: boolean;
}) {
  const { t } = useTranslation();
  const normalizedFrontendVersion = normalizeVersion(frontendVersion);
  const normalizedBackendVersion = backendVersion ? normalizeVersion(backendVersion) : null;
  const releaseLink = normalizedBackendVersion
    ? `${repositoryUrl}/releases/tag/${normalizedBackendVersion}`
    : null;
  const sameVersion = normalizedBackendVersion === normalizedFrontendVersion;
  const version =
    sameVersion && normalizedBackendVersion ? (
      <a
        className="app-link font-mono"
        href={releaseLink ?? undefined}
        target="_blank"
        rel="noreferrer"
      >
        {normalizedBackendVersion}
      </a>
    ) : normalizedBackendVersion ? (
      <span className="inline-flex items-center gap-2">
        <span>{t("app.footer.newVersionAvailable")} </span>
        <span className="font-mono text-base-content/60">
          <del style={{ textDecorationColor: "currentColor" }}>{normalizedFrontendVersion}</del>
        </span>
        <span aria-hidden> → </span>
        <a
          className="app-link font-mono"
          href={releaseLink ?? undefined}
          target="_blank"
          rel="noreferrer"
        >
          {normalizedBackendVersion}
        </a>
      </span>
    ) : (
      <span className="font-mono">{normalizedFrontendVersion}</span>
    );
  return (
    <footer
      className="border-t border-base-300/75 bg-base-100/80 text-sm text-base-content/70 backdrop-blur"
      data-testid="app-footer"
    >
      <div
        className="app-shell-boundary flex flex-wrap items-center justify-between gap-3 px-4 py-3"
        data-testid="app-footer-inner"
      >
        <span>{t("app.footer.copyright")}</span>
        <div className="flex flex-wrap items-center gap-4">
          <a
            className="app-link flex items-center gap-1"
            href={repositoryUrl}
            target="_blank"
            rel="noreferrer"
            aria-label={t("app.footer.githubAria")}
          >
            <AppIcon name="github" className="h-4 w-4" aria-hidden />
            <span>GitHub</span>
          </a>
          <div className="flex items-center gap-2">
            {version}
            {isBackendLoading && !sameVersion ? (
              <span className="flex items-center gap-1 text-base-content/60" aria-live="polite">
                <AppIcon name="loading" className="h-3 w-3 animate-spin" aria-hidden />
                <span className="sr-only">{t("app.footer.loadingVersion")}</span>
              </span>
            ) : null}
          </div>
        </div>
      </div>
    </footer>
  );
}
