//! [`range_assign`]: the "range" consumer-group partition assignment
//! strategy, computed by whichever member `JoinGroup` elects leader and
//! sent back to the coordinator via `SyncGroup`.
//!
//! `rusty_kafka` implements the full wire protocol for consumer-group
//! coordination but explicitly leaves the partition-assignment
//! *decision* itself as policy for its callers to supply (see that
//! crate's own module doc) -- the same `CreateTopics` (raw protocol,
//! `rusty_kafka`) vs. `TopicManager` (naming-convention policy,
//! `rusty-meshed-sdk`) split this crate already uses elsewhere. This
//! module is that policy for [`crate::consumer::DataProductConsumerBase`],
//! mirroring real Kafka's built-in `RangeAssignor`: per topic, sort the
//! subscribed members by member ID, then split that topic's partitions
//! into contiguous ranges, one per member, as evenly as possible (any
//! remainder partitions go to the first `remainder` members in sorted
//! order). `"range"` is also the `protocol_name` this crate's
//! `JoinGroupProtocol` declares, matching the algorithm actually run
//! here.

use std::collections::BTreeMap;

/// Computes a full consumer group's partition assignment via the
/// "range" strategy.
///
/// `subscriptions` is each member's ID paired with the topics it
/// declared (decoded from a `JoinGroupResponse`'s per-member
/// `ConsumerProtocolSubscription`, only populated in the leader's own
/// response -- see `rusty_kafka::protocol::consumer_protocol`).
/// `partition_counts` gives each subscribed topic's total partition
/// count (from a `Metadata` call).
///
/// Returns every member's own `(topic, partitions)` assignment --
/// exactly what [`crate::consumer`] encodes per member via
/// `encode_assignment` for `SyncGroupRequest::assignments`. A member
/// not subscribed to a topic never appears in that topic's split, even
/// if it is a member of the group.
pub fn range_assign(
    subscriptions: &[(String, Vec<String>)],
    partition_counts: &BTreeMap<String, i32>,
) -> BTreeMap<String, Vec<(String, Vec<i32>)>> {
    let mut assignment: BTreeMap<String, Vec<(String, Vec<i32>)>> = subscriptions
        .iter()
        .map(|(member_id, _)| (member_id.clone(), Vec::new()))
        .collect();

    let mut topics: Vec<&String> = partition_counts.keys().collect();
    topics.sort();

    for topic in topics {
        let partition_count = *partition_counts.get(topic).unwrap_or(&0);
        let mut members: Vec<&String> = subscriptions
            .iter()
            .filter(|(_, subscribed_topics)| subscribed_topics.contains(topic))
            .map(|(member_id, _)| member_id)
            .collect();
        members.sort();
        if members.is_empty() || partition_count == 0 {
            continue;
        }

        let member_count = members.len() as i32;
        let partitions_per_member = partition_count / member_count;
        let remainder = partition_count % member_count;
        let mut next_partition = 0;
        for (index, member_id) in members.iter().enumerate() {
            let count = partitions_per_member + if (index as i32) < remainder { 1 } else { 0 };
            let partitions: Vec<i32> = (next_partition..next_partition + count).collect();
            next_partition += count;
            if let Some(entry) = assignment.get_mut(*member_id) {
                entry.push((topic.clone(), partitions));
            }
        }
    }

    assignment
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_member_gets_every_partition() {
        let subscriptions = vec![("consumer-1".to_string(), vec!["t".to_string()])];
        let mut counts = BTreeMap::new();
        counts.insert("t".to_string(), 3);

        let assignment = range_assign(&subscriptions, &counts);
        assert_eq!(
            assignment["consumer-1"],
            vec![("t".to_string(), vec![0, 1, 2])]
        );
    }

    #[test]
    fn two_members_split_partitions_with_the_remainder_to_the_first() {
        let subscriptions = vec![
            ("consumer-1".to_string(), vec!["t".to_string()]),
            ("consumer-2".to_string(), vec!["t".to_string()]),
        ];
        let mut counts = BTreeMap::new();
        counts.insert("t".to_string(), 3);

        let assignment = range_assign(&subscriptions, &counts);
        assert_eq!(
            assignment["consumer-1"],
            vec![("t".to_string(), vec![0, 1])]
        );
        assert_eq!(assignment["consumer-2"], vec![("t".to_string(), vec![2])]);
    }

    #[test]
    fn members_split_evenly_when_partitions_divide_cleanly() {
        let subscriptions = vec![
            ("consumer-1".to_string(), vec!["t".to_string()]),
            ("consumer-2".to_string(), vec!["t".to_string()]),
        ];
        let mut counts = BTreeMap::new();
        counts.insert("t".to_string(), 4);

        let assignment = range_assign(&subscriptions, &counts);
        assert_eq!(
            assignment["consumer-1"],
            vec![("t".to_string(), vec![0, 1])]
        );
        assert_eq!(
            assignment["consumer-2"],
            vec![("t".to_string(), vec![2, 3])]
        );
    }

    #[test]
    fn a_member_not_subscribed_to_a_topic_gets_none_of_it() {
        let subscriptions = vec![
            ("consumer-1".to_string(), vec!["t1".to_string()]),
            ("consumer-2".to_string(), vec!["t2".to_string()]),
        ];
        let mut counts = BTreeMap::new();
        counts.insert("t1".to_string(), 2);
        counts.insert("t2".to_string(), 2);

        let assignment = range_assign(&subscriptions, &counts);
        assert_eq!(
            assignment["consumer-1"],
            vec![("t1".to_string(), vec![0, 1])]
        );
        assert_eq!(
            assignment["consumer-2"],
            vec![("t2".to_string(), vec![0, 1])]
        );
    }

    #[test]
    fn a_topic_with_no_subscribers_is_skipped() {
        let subscriptions: Vec<(String, Vec<String>)> = vec![];
        let mut counts = BTreeMap::new();
        counts.insert("t".to_string(), 3);

        let assignment = range_assign(&subscriptions, &counts);
        assert!(assignment.is_empty());
    }
}
