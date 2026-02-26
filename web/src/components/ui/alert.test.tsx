import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { Alert } from './alert'

describe('Alert variants', () => {
  it('keeps warning variant on supported opacity token', () => {
    const html = renderToStaticMarkup(<Alert variant="warning">warning</Alert>)

    expect(html).toContain('bg-warning/15')
    expect(html).not.toContain('bg-warning/14')
  })

  it('keeps error variant on supported opacity token', () => {
    const html = renderToStaticMarkup(<Alert variant="error">error</Alert>)

    expect(html).toContain('bg-error/15')
    expect(html).not.toContain('bg-error/12')
  })
})
