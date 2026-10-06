/** Calendar subscriptions as `subscription` client documents. */
import { createDocStore } from '@/lib/docStore'
import { asSubscriptionBody } from './logic'

const { useDocs, reset } = createDocStore('subscription', asSubscriptionBody, 'subscription')
export const useSubscriptions = useDocs
export const resetSubscriptionsStore = reset
