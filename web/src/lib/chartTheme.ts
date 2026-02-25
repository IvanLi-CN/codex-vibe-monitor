import type { ThemeMode } from '../theme'

export type MetricPaletteKey = 'totalCount' | 'totalCost' | 'totalTokens'

export interface ChartBaseTokens {
  axisText: string
  gridLine: string
  tooltipBg: string
  tooltipBorder: string
}

export interface ChartStatusTokens {
  success: string
  failure: string
}

type ColorScale = {
  light: string
  dark: string
}

type MetricColorTable = Record<MetricPaletteKey, ColorScale>

type HeatmapScaleTable = Record<MetricPaletteKey, { light: string[]; dark: string[] }>

const METRIC_ACCENTS: MetricColorTable = {
  totalCount: { light: '#1d4ed8', dark: '#38bdf8' },
  totalCost: { light: '#c2410c', dark: '#f59e0b' },
  totalTokens: { light: '#0f766e', dark: '#2dd4bf' },
}

const HEATMAP_LEVELS: HeatmapScaleTable = {
  totalCount: {
    light: ['#e5eaf0', '#d3e3fb', '#a9c8f9', '#6ca4f2', '#387be5'],
    dark: ['#1f2937', '#1e3a5f', '#175582', '#0a78ad', '#14a3df'],
  },
  totalCost: {
    light: ['#e5eaf0', '#fce4c6', '#f9c989', '#f4a44a', '#e47a18'],
    dark: ['#1f2937', '#593a1c', '#7f4e1e', '#ab6119', '#d97706'],
  },
  totalTokens: {
    light: ['#e5eaf0', '#cdeee7', '#9adfce', '#56c9b2', '#1ea88e'],
    dark: ['#1f2937', '#1b4f47', '#176a60', '#148474', '#2dd4bf'],
  },
}

const CHART_BASE_TOKENS: Record<ThemeMode, ChartBaseTokens> = {
  light: {
    axisText: '#4b5563',
    gridLine: '#d8e0ea',
    tooltipBg: '#ffffff',
    tooltipBorder: '#cbd5e1',
  },
  dark: {
    axisText: '#9ca3af',
    gridLine: '#334155',
    tooltipBg: '#111827',
    tooltipBorder: '#475569',
  },
}

const CHART_STATUS_TOKENS: Record<ThemeMode, ChartStatusTokens> = {
  light: {
    success: '#16a34a',
    failure: '#dc2626',
  },
  dark: {
    success: '#22c55e',
    failure: '#f87171',
  },
}

const PIE_PALETTE: Record<ThemeMode, string[]> = {
  light: ['#dc2626', '#f97316', '#f59e0b', '#eab308', '#16a34a', '#0d9488', '#0284c7', '#2563eb', '#1d4ed8'],
  dark: ['#ef4444', '#fb923c', '#fbbf24', '#facc15', '#22c55e', '#2dd4bf', '#22d3ee', '#60a5fa', '#818cf8'],
}

export function metricAccent(metric: MetricPaletteKey, themeMode: ThemeMode): string {
  return METRIC_ACCENTS[metric][themeMode]
}

export function heatmapLevels(metric: MetricPaletteKey, themeMode: ThemeMode): string[] {
  return HEATMAP_LEVELS[metric][themeMode]
}

export function calendarPalette(metric: MetricPaletteKey, themeMode: ThemeMode): string[] {
  return HEATMAP_LEVELS[metric][themeMode]
}

export function chartBaseTokens(themeMode: ThemeMode): ChartBaseTokens {
  return CHART_BASE_TOKENS[themeMode]
}

export function chartStatusTokens(themeMode: ThemeMode): ChartStatusTokens {
  return CHART_STATUS_TOKENS[themeMode]
}

export function piePalette(themeMode: ThemeMode): string[] {
  return PIE_PALETTE[themeMode]
}

export function withOpacity(hex: string, opacity: number): string {
  const normalized = hex.replace('#', '')
  if (normalized.length !== 6) return hex
  const safeOpacity = Math.max(0, Math.min(1, opacity))
  const alpha = Math.round(safeOpacity * 255)
    .toString(16)
    .padStart(2, '0')
  return `#${normalized}${alpha}`
}
