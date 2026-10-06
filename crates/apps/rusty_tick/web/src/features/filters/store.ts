/** Saved filters as `filter` client documents. */
import { createDocStore } from '@/lib/docStore'
import { asFilterBody } from './logic'

const { useDocs, reset } = createDocStore('filter', asFilterBody, 'filter')
export const useFilters = useDocs
export const resetFiltersStore = reset
