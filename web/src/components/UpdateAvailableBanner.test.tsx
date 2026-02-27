import { Children, isValidElement, type ReactElement, type ReactNode } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { Button } from './ui/button'
import { UpdateAvailableBanner } from './UpdateAvailableBanner'

type ButtonElement = ReactElement<{
  children?: ReactNode
  onClick?: () => void
  disabled?: boolean
}>

function collectButtons(node: ReactNode, buttons: ButtonElement[] = []): ButtonElement[] {
  if (!isValidElement<{ children?: ReactNode }>(node)) {
    return buttons
  }

  if (node.type === Button) {
    buttons.push(node as ButtonElement)
  }

  Children.forEach(node.props.children, child => {
    collectButtons(child, buttons)
  })

  return buttons
}

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

  it('binds refresh and dismiss buttons to provided callbacks', () => {
    const onReload = vi.fn()
    const onDismiss = vi.fn()

    const tree = UpdateAvailableBanner({
      currentVersion: '0.10.2',
      availableVersion: '0.10.4',
      onReload,
      onDismiss,
      labels: {
        available: '有新版本可用：',
        refresh: '立即刷新',
        later: '稍后',
      },
    })

    const buttons = collectButtons(tree)
    const refreshButton = buttons.find(button => button.props.children === '立即刷新')
    const laterButton = buttons.find(button => button.props.children === '稍后')

    expect(refreshButton).toBeDefined()
    expect(laterButton).toBeDefined()

    expect(refreshButton?.props.onClick).toBe(onReload)
    expect(laterButton?.props.onClick).toBe(onDismiss)
    expect(refreshButton?.props.disabled).not.toBe(true)
    expect(laterButton?.props.disabled).not.toBe(true)
  })
})
