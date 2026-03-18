import type { Meta, StoryObj } from '@storybook/react-vite'
import { useEffect, useRef, useState } from 'react'
import { AccountTagField } from './AccountTagField'
import type { CreateTagPayload, TagDetail, TagSummary, UpdateTagPayload } from '../lib/api'

const baseTags: TagSummary[] = [
  {
    id: 1,
    name: 'vip-routing',
    routingRule: {
      guardEnabled: false,
      lookbackHours: null,
      maxConversations: null,
      allowCutOut: true,
      allowCutIn: true,
    },
    accountCount: 3,
    groupCount: 1,
    updatedAt: '2026-03-14T15:20:00.000Z',
  },
  {
    id: 2,
    name: 'handoff-blocked',
    routingRule: {
      guardEnabled: true,
      lookbackHours: 4,
      maxConversations: 8,
      allowCutOut: false,
      allowCutIn: true,
    },
    accountCount: 2,
    groupCount: 2,
    updatedAt: '2026-03-14T12:00:00.000Z',
  },
]

const labels = {
  label: 'Tags',
  add: 'Add tag',
  empty: 'No tags linked yet.',
  searchPlaceholder: 'Search tags',
  searchEmpty: 'No matching tags.',
  createInline: (value: string) => (value ? `Create "${value}"` : 'Create new tag'),
  selectedFromCurrentPage: 'New',
  remove: 'Unlink tag',
  deleteAndRemove: 'Delete and unlink',
  edit: 'Edit routing rule',
  createTitle: 'Create tag',
  editTitle: 'Edit tag',
  dialogDescription: 'Configure the routing policy bound to this tag.',
  name: 'Name',
  namePlaceholder: 'vip-routing',
  guardEnabled: 'Conversation guard',
  lookbackHours: 'Lookback hours',
  maxConversations: 'Max conversations',
  allowCutOut: 'Allow cut out',
  allowCutIn: 'Allow cut in',
  cancel: 'Cancel',
  save: 'Save',
  createAction: 'Create',
  validation: 'Use positive integers for the guard values.',
}

function StorySurface({ children }: { children: React.ReactNode }) {
  return (
    <div className="min-h-screen bg-base-200 px-6 py-8 text-base-content">
      <div className="mx-auto max-w-6xl">{children}</div>
    </div>
  )
}

function createDetailFromSummary(summary: TagSummary): TagDetail {
  return { ...summary }
}

function FieldHarness({
  pageCreatedTagIds = [],
  initialSelectedTagIds = [1, 2],
  autoOpenTarget,
}: {
  pageCreatedTagIds?: number[]
  initialSelectedTagIds?: number[]
  autoOpenTarget?: 'picker' | 'chip-menu'
}) {
  const rootRef = useRef<HTMLDivElement | null>(null)
  const [tags, setTags] = useState<TagSummary[]>(baseTags)
  const [selectedTagIds, setSelectedTagIds] = useState<number[]>(initialSelectedTagIds)

  useEffect(() => {
    if (!autoOpenTarget) return
    const frame = window.requestAnimationFrame(() => {
      const root = rootRef.current
      if (!root) return
      const selector =
        autoOpenTarget === 'picker'
          ? `button[aria-label="${labels.add}"]`
          : 'button[aria-label="vip-routing more actions"]'
      root.querySelector<HTMLButtonElement>(selector)?.click()
    })
    return () => window.cancelAnimationFrame(frame)
  }, [autoOpenTarget])

  const createTag = async (payload: CreateTagPayload) => {
    const detail: TagDetail = {
      id: Math.max(...tags.map((item) => item.id)) + 1,
      name: payload.name,
      routingRule: {
        guardEnabled: payload.guardEnabled,
        lookbackHours: payload.lookbackHours ?? null,
        maxConversations: payload.maxConversations ?? null,
        allowCutOut: payload.allowCutOut,
        allowCutIn: payload.allowCutIn,
      },
      accountCount: 0,
      groupCount: 0,
      updatedAt: '2026-03-14T16:00:00.000Z',
    }
    setTags((current) => [...current, detail])
    return detail
  }

  const updateTag = async (tagId: number, payload: UpdateTagPayload) => {
    let updated: TagDetail | null = null
    setTags((current) =>
      current.map((item) => {
        if (item.id !== tagId) return item
        const next: TagDetail = {
          ...item,
          name: payload.name ?? item.name,
          routingRule: {
            guardEnabled: payload.guardEnabled ?? item.routingRule.guardEnabled,
            lookbackHours: payload.lookbackHours ?? null,
            maxConversations: payload.maxConversations ?? null,
            allowCutOut: payload.allowCutOut ?? item.routingRule.allowCutOut,
            allowCutIn: payload.allowCutIn ?? item.routingRule.allowCutIn,
          },
          updatedAt: '2026-03-14T16:30:00.000Z',
        }
        updated = next
        return next
      }),
    )
    return updated ?? createDetailFromSummary(tags[0]!)
  }

  const deleteTag = async (tagId: number) => {
    setTags((current) => current.filter((item) => item.id !== tagId))
    setSelectedTagIds((current) => current.filter((value) => value !== tagId))
  }

  return (
    <div ref={rootRef} className="rounded-[1.8rem] border border-base-300/70 bg-base-100/75 p-6">
      <AccountTagField
        tags={tags}
        selectedTagIds={selectedTagIds}
        writesEnabled
        pageCreatedTagIds={pageCreatedTagIds}
        labels={labels}
        onChange={setSelectedTagIds}
        onCreateTag={createTag}
        onUpdateTag={updateTag}
        onDeleteTag={deleteTag}
      />
    </div>
  )
}

function ShowcaseCard({
  title,
  description,
  children,
}: {
  title: string
  description: string
  children: React.ReactNode
}) {
  return (
    <section className="rounded-[1.6rem] border border-base-300/70 bg-base-100/75 p-5 shadow-sm">
      <div className="mb-4 space-y-1">
        <h3 className="text-sm font-semibold uppercase tracking-[0.14em] text-base-content/70">{title}</h3>
        <p className="text-sm text-base-content/60">{description}</p>
      </div>
      {children}
    </section>
  )
}

function OverviewHarness() {
  return (
    <StorySurface>
      <div className="space-y-6">
        <div className="max-w-2xl space-y-2">
          <h2 className="text-xl font-semibold text-base-content">Account Tag Field Overview</h2>
          <p className="text-sm text-base-content/65">
            聚合查看空态、默认态、当前页新建 tag、添加气泡展开和 tag 菜单展开，方便一眼核对最终效果。
          </p>
        </div>

        <div className="grid gap-4 xl:grid-cols-2">
          <ShowcaseCard title="Default" description="标准多选展示，右侧保留添加触发器。">
            <FieldHarness />
          </ShowcaseCard>

          <ShowcaseCard title="Empty" description="未选择任何 tag 时，空态文案与添加触发器同列显示。">
            <FieldHarness initialSelectedTagIds={[]} />
          </ShowcaseCard>

          <ShowcaseCard title="Page-Created Tag" description="当前页刚创建的 tag 继续带有 New 标记。">
            <FieldHarness pageCreatedTagIds={[2]} />
          </ShowcaseCard>

          <ShowcaseCard title="Picker Open" description="自动展开添加气泡，直接检查搜索与多选弹层尺寸。">
            <FieldHarness autoOpenTarget="picker" />
          </ShowcaseCard>

          <ShowcaseCard
            title="Chip Menu Open"
            description="自动展开 tag 上下文菜单，确认紧凑菜单和 chip 尺寸关系。"
          >
            <FieldHarness autoOpenTarget="chip-menu" />
          </ShowcaseCard>
        </div>
      </div>
    </StorySurface>
  )
}

const meta = {
  title: 'Account Pool/Components/Account Tag Field',
  component: AccountTagField,
  tags: ['autodocs'],
  parameters: {
    layout: 'fullscreen',
    docs: {
      description: {
        component:
          '上游账号详情与创建页共用的内联 tag 选择字段。已选 tag 以内联 chips 形式展示在同一个输入式容器中，尾部通过气泡触发器完成搜索、多选与创建；每个已选 tag 继续复用独立的上下文菜单芯片交互。',
      },
    },
  },
  args: {
    tags: baseTags,
    selectedTagIds: [1],
    writesEnabled: true,
    labels,
    onChange: () => undefined,
    onCreateTag: async () => createDetailFromSummary(baseTags[0]!),
    onUpdateTag: async () => createDetailFromSummary(baseTags[0]!),
    onDeleteTag: async () => undefined,
  },
} satisfies Meta<typeof AccountTagField>

export default meta

type Story = StoryObj<typeof meta>

export const Overview: Story = {
  render: () => <OverviewHarness />,
}

export const Default: Story = {
  render: () => (
    <StorySurface>
      <FieldHarness />
    </StorySurface>
  ),
}

export const Empty: Story = {
  render: () => (
    <StorySurface>
      <FieldHarness initialSelectedTagIds={[]} />
    </StorySurface>
  ),
}

export const WithPageCreatedTag: Story = {
  render: () => (
    <StorySurface>
      <FieldHarness pageCreatedTagIds={[2]} />
    </StorySurface>
  ),
}
