import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { SuccessFailureTooltipContent } from './SuccessFailureChart'

const labels = {
  failure: '失败',
  success: '成功',
  successRate: '成功率',
  firstByteAvg: '首字耗时均值',
  firstByteP95: '首字耗时 P95',
}

const numberFormatter = new Intl.NumberFormat('zh-CN', { maximumFractionDigits: 0 })
const percentFormatter = new Intl.NumberFormat('zh-CN', { maximumFractionDigits: 1 })
const latencyFormatter = new Intl.NumberFormat('zh-CN', { maximumFractionDigits: 1 })

describe('SuccessFailureTooltipContent', () => {
  it('renders mapped success/failure and latency metrics for a populated bucket', () => {
    const html = renderToStaticMarkup(
      <SuccessFailureTooltipContent
        label="2026-02-27 16:15"
        datum={{
          label: '2026-02-27 16:15',
          success: 164,
          failure: 55,
          successRate: 164 / (164 + 55),
          firstByteSampleCount: 164,
          firstByteAvgMs: 320.4,
          firstByteP95Ms: 690.8,
        }}
        labels={labels}
        noValueLabel="—"
        numberFormatter={numberFormatter}
        percentFormatter={percentFormatter}
        latencyFormatter={latencyFormatter}
        tooltipBg="#fff"
        tooltipBorder="#ddd"
        axisText="#333"
      />,
    )

    expect(html).toContain('失败')
    expect(html).toContain('55')
    expect(html).toContain('成功')
    expect(html).toContain('164')
    expect(html).toContain('成功率')
    expect(html).toContain('74.9%')
    expect(html).toContain('首字耗时均值')
    expect(html).toContain('320.4 ms')
    expect(html).toContain('首字耗时 P95')
    expect(html).toContain('690.8 ms')
  })

  it('falls back to em dash when bucket has no valid first-byte samples', () => {
    const html = renderToStaticMarkup(
      <SuccessFailureTooltipContent
        label="2026-02-27 16:30"
        datum={{
          label: '2026-02-27 16:30',
          success: 0,
          failure: 0,
          successRate: null,
          firstByteSampleCount: 0,
          firstByteAvgMs: null,
          firstByteP95Ms: null,
        }}
        labels={labels}
        noValueLabel="—"
        numberFormatter={numberFormatter}
        percentFormatter={percentFormatter}
        latencyFormatter={latencyFormatter}
        tooltipBg="#fff"
        tooltipBorder="#ddd"
        axisText="#333"
      />,
    )

    expect(html).toContain('成功率')
    expect(html).toContain('—')
    expect(html).toContain('首字耗时均值')
    expect(html).toContain('首字耗时 P95')
  })
})
