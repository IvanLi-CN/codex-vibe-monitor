import { useCallback, useEffect, useState } from 'react'
import {
  createTag,
  deleteTag,
  fetchTags,
  updateTag,
  type CreateTagPayload,
  type FetchTagsQuery,
  type TagDetail,
  type TagSummary,
  type UpdateTagPayload,
} from '../lib/api'

export function usePoolTags(initialQuery?: FetchTagsQuery) {
  const [items, setItems] = useState<TagSummary[]>([])
  const [writesEnabled, setWritesEnabled] = useState(true)
  const [isLoading, setIsLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [query, setQuery] = useState<FetchTagsQuery>(initialQuery ?? {})

  const load = useCallback(async (nextQuery?: FetchTagsQuery) => {
    setIsLoading(true)
    try {
      const resolvedQuery = nextQuery ?? query
      const response = await fetchTags(resolvedQuery)
      setItems(response.items)
      setWritesEnabled(response.writesEnabled)
      setError(null)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setIsLoading(false)
    }
  }, [query])

  useEffect(() => {
    void load(query)
  }, [load, query])

  const refresh = useCallback(async () => {
    await load(query)
  }, [load, query])

  const updateQuery = useCallback((nextQuery: FetchTagsQuery) => {
    setQuery(nextQuery)
  }, [])

  const createOne = useCallback(async (payload: CreateTagPayload): Promise<TagDetail> => {
    const detail = await createTag(payload)
    await load(query)
    return detail
  }, [load, query])

  const updateOne = useCallback(async (tagId: number, payload: UpdateTagPayload): Promise<TagDetail> => {
    const detail = await updateTag(tagId, payload)
    await load(query)
    return detail
  }, [load, query])

  const removeOne = useCallback(async (tagId: number) => {
    await deleteTag(tagId)
    await load(query)
  }, [load, query])

  return {
    items,
    writesEnabled,
    isLoading,
    error,
    query,
    refresh,
    updateQuery,
    createTag: createOne,
    updateTag: updateOne,
    deleteTag: removeOne,
  }
}
