/** Countdowns as `countdown` client documents. */
import { createDocStore } from '@/lib/docStore'
import { asCountdownBody } from './logic'

const { useDocs, reset } = createDocStore('countdown', asCountdownBody, 'countdown')
export const useCountdowns = useDocs
export const resetCountdownsStore = reset
