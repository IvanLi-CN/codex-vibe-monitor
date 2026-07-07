import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { SuccessFailureTooltipContent } from './SuccessFailureChart'

const labels = {
  failure: '失败',
  success: '成功',
  successRate: '成功率',
  firstResponseByteTotalAvg: '首字总耗时均值',
  firstResponseByteTotalP95: '首字总耗时 P95',
}

const numberFormatter = new Intl.NumberFormat('zh-CN', { maximumFractionDigits: 0 })
const percentFormatter = new Intl.NumberFormat('zh-CN', { maximumFractionDigits: 1 })
const latencyMsFormatter = new Intl.NumberFormat('zh-CN', { maximumFractionDigits: 1 })

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
          firstResponseByteTotalSampleCount: 164,
          firstResponseByteTotalAvgMs: 320.4,
          firstResponseByteTotalP95Ms: 690.8,
        }}
        labels={labels}
        noValueLabel="—"
        numberFormatter={numberFormatter}
        percentFormatter={percentFormatter}
        latencyMsFormatter={latencyMsFormatter}
        localeTag="zh-CN"
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
    expect(html).toContain('首字总耗时均值')
    expect(html).toContain('320.4 ms')
    expect(html).toContain('首字总耗时 P95')
    expect(html).toContain('690.8 ms')
  })

  it('formats first-response-byte totals in seconds once they cross one second', () => {
    const html = renderToStaticMarkup(
      <SuccessFailureTooltipContent
        label="2026-03-26 20:30"
        datum={{
          label: '2026-03-26 20:30',
          success: 9,
          failure: 1,
          successRate: 0.9,
          firstResponseByteTotalSampleCount: 10,
          firstResponseByteTotalAvgMs: 43_890,
          firstResponseByteTotalP95Ms: 52_340,
        }}
        labels={labels}
        noValueLabel="—"
        numberFormatter={numberFormatter}
        percentFormatter={percentFormatter}
        latencyMsFormatter={latencyMsFormatter}
        localeTag="zh-CN"
        tooltipBg="#fff"
        tooltipBorder="#ddd"
        axisText="#333"
      />,
    )

    expect(html).toContain('43.89 s')
    expect(html).toContain('52.34 s')
  })

  it('falls back to em dash when bucket has no valid first-response-byte-total samples', () => {
    const html = renderToStaticMarkup(
      <SuccessFailureTooltipContent
        label="2026-02-27 16:30"
        datum={{
          label: '2026-02-27 16:30',
          success: 0,
          failure: 0,
          successRate: null,
          firstResponseByteTotalSampleCount: 0,
          firstResponseByteTotalAvgMs: null,
          firstResponseByteTotalP95Ms: null,
        }}
        labels={labels}
        noValueLabel="—"
        numberFormatter={numberFormatter}
        percentFormatter={percentFormatter}
        latencyMsFormatter={latencyMsFormatter}
        localeTag="zh-CN"
        tooltipBg="#fff"
        tooltipBorder="#ddd"
        axisText="#333"
      />,
    )

    expect(html).toContain('成功率')
    expect(html).toContain('—')
    expect(html).toContain('首字总耗时均值')
    expect(html).toContain('首字总耗时 P95')
  })
})
