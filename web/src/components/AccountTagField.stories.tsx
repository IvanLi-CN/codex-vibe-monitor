import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
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
      <div className="mx-auto max-w-4xl">{children}</div>
    </div>
  )
}

function createDetailFromSummary(summary: TagSummary): TagDetail {
  return { ...summary }
}

function FieldHarness({
  pageCreatedTagIds = [],
  initialSelectedTagIds = [1, 2],
}: {
  pageCreatedTagIds?: number[]
  initialSelectedTagIds?: number[]
}) {
  const [tags, setTags] = useState<TagSummary[]>(baseTags)
  const [selectedTagIds, setSelectedTagIds] = useState<number[]>(initialSelectedTagIds)

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
    <StorySurface>
      <div className="rounded-[1.8rem] border border-base-300/70 bg-base-100/75 p-6">
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

export const Default: Story = {
  render: () => <FieldHarness />,
}

export const Empty: Story = {
  render: () => <FieldHarness initialSelectedTagIds={[]} />,
}

export const WithPageCreatedTag: Story = {
  render: () => <FieldHarness pageCreatedTagIds={[2]} />,
}
