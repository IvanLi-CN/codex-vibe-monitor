import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { UpdateAvailableBanner } from './UpdateAvailableBanner'

describe('UpdateAvailableBanner', () => {
  it('renders update text, versions, and action labels', () => {
    const html = renderToStaticMarkup(
      <UpdateAvailableBanner
        currentVersion="0.10.2"
        availableVersion="0.10.4"
        onReload={vi.fn()}
        onDismiss={vi.fn()}
        labels={{
          available: '有新版本可用：',
          refresh: '立即刷新',
          later: '稍后',
        }}
      />,
    )

    expect(html).toContain('有新版本可用：')
    expect(html).toContain('0.10.2')
    expect(html).toContain('0.10.4')
    expect(html).toContain('→')
    expect(html).toContain('立即刷新')
    expect(html).toContain('稍后')
  })

  it('includes a11y status attributes', () => {
    const html = renderToStaticMarkup(
      <UpdateAvailableBanner
        currentVersion="0.10.2"
        availableVersion="0.10.4"
        onReload={vi.fn()}
        onDismiss={vi.fn()}
        labels={{
          available: 'A new version is available:',
          refresh: 'Refresh now',
          later: 'Later',
        }}
      />,
    )

    expect(html).toContain('role="status"')
    expect(html).toContain('aria-live="polite"')
  })

  it('uses readable primary styles and avoids low-contrast info-content token', () => {
    const html = renderToStaticMarkup(
      <UpdateAvailableBanner
        currentVersion="0.10.2"
        availableVersion="0.10.4"
        onReload={vi.fn()}
        onDismiss={vi.fn()}
        labels={{
          available: 'A new version is available:',
          refresh: 'Refresh now',
          later: 'Later',
        }}
      />,
    )

    expect(html).toContain('bg-primary/10')
    expect(html).toContain('border-primary/35')
    expect(html).toContain('text-base-content')
    expect(html).not.toContain('text-info-content')
  })
})
