//! Change feed: one [`ChangeEvent`] per committed write.
//!
//! The embedded database publishes an event after every successful storage
//! commit that changes data or the catalog, while it still holds the writer
//! lock, so events arrive in commit (revision) order. Failed and conflicted
//! commits publish nothing, and neither do the backfills run while opening a
//! database.
//!
//! Delivery goes through a bounded broadcast: a subscriber that falls more
//! than the feed capacity behind loses the oldest events and receives a
//! [`ChangeFeedItem::Lagged`] notice instead. The feed does not replay
//! history: a subscription only sees commits made after it was created, so
//! clients that need a consistent view read the current state (see
//! `EmbeddedDb::current_revision`) and then apply events with a higher
//! revision. Replay of past revisions could later be provided by a
//! log-structured engine.

use std::collections::BTreeSet;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, ready};

use futures::stream::BoxStream;
use futures::{Stream, StreamExt as _};
use semantic_data::value::{DateTime, Object};

/// Events retained per subscriber before it is considered lagging.
pub const DEFAULT_CHANGE_FEED_CAPACITY: usize = 1024;

/// Stream of change feed items returned by `Backend::subscribe_changes`.
pub type ChangeStream = Pin<Box<dyn Stream<Item = ChangeFeedItem> + Send>>;

/// What produced a committed write.
#[derive(facet::Facet, Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum ChangeSource {
    /// A batch or a single-statement write, including predicate updates and
    /// deletes.
    Batch,
    /// The commit of an interactive transaction.
    Transaction,
    /// A DDL change.
    Ddl,
    /// A package registration and its migrations.
    Migration,
    /// Database maintenance, such as validation activation.
    Maintenance,
}

#[derive(facet::Facet, Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum ChangeKind {
    Created,
    Updated,
    Deleted,
}

/// The net change of one row in a commit.
#[derive(facet::Facet, Debug, Clone, PartialEq)]
pub struct ChangedEntity {
    pub collection: String,
    pub id: String,
    pub kind: ChangeKind,
    /// The row before the commit; only with
    /// [`ChangeSubscriptionOptions::include_payloads`].
    pub before: Option<Object>,
    /// The row after the commit; only with
    /// [`ChangeSubscriptionOptions::include_payloads`].
    pub after: Option<Object>,
}

impl ChangedEntity {
    /// The change from `before` to `after`, or `None` if the row neither
    /// existed before nor after.
    pub(crate) fn new(
        collection: String,
        id: String,
        before: Option<Object>,
        after: Option<Object>,
    ) -> Option<Self> {
        let kind = match (&before, &after) {
            (None, None) => return None,
            (None, Some(_)) => ChangeKind::Created,
            (Some(_), Some(_)) => ChangeKind::Updated,
            (Some(_), None) => ChangeKind::Deleted,
        };
        Some(Self {
            collection,
            id,
            kind,
            before,
            after,
        })
    }

    fn without_payloads(&self) -> Self {
        Self {
            collection: self.collection.clone(),
            id: self.id.clone(),
            kind: self.kind,
            before: None,
            after: None,
        }
    }
}

/// One committed write.
#[derive(facet::Facet, Debug, Clone, PartialEq)]
pub struct ChangeEvent {
    /// Storage revision created by the commit. Revisions increase strictly
    /// with every commit but are not contiguous in a subscription: commits
    /// hidden by its filter are not delivered.
    pub revision: u64,
    pub committed_at: DateTime,
    pub source: ChangeSource,
    /// Net row changes, ordered by collection and id.
    pub changes: Vec<ChangedEntity>,
    /// Catalog version after the commit.
    pub catalog_version: u64,
    /// Whether the commit installed a new catalog.
    pub catalog_changed: bool,
}

/// An item of a change subscription.
#[derive(facet::Facet, Debug, Clone, PartialEq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum ChangeFeedItem {
    Event(ChangeEvent),
    /// The subscriber fell behind and `skipped` commits were dropped (the
    /// count may include commits its filter would have hidden). Delivery
    /// resumes with the commit at `resume_revision`: every later commit is
    /// delivered again, earlier undelivered ones are lost. Clients re-read
    /// the state they track to recover.
    Lagged {
        skipped: u64,
        resume_revision: u64,
    },
}

/// What a change subscription delivers.
#[derive(facet::Facet, Debug, Clone, PartialEq, Eq, Default)]
pub struct ChangeSubscriptionOptions {
    /// Include the before/after rows of every change; otherwise only
    /// collections, ids and kinds are delivered.
    pub include_payloads: bool,
    /// Include changes to internal collections.
    pub include_internal: bool,
    /// Only deliver changes to these collections; empty delivers all.
    /// Filtered subscriptions skip commits without matching changes,
    /// including catalog-only commits.
    pub collections: Vec<String>,
}

impl ChangeSubscriptionOptions {
    pub fn with_payloads(mut self) -> Self {
        self.include_payloads = true;
        self
    }

    pub fn with_internal(mut self) -> Self {
        self.include_internal = true;
        self
    }

    pub fn with_collections<I, C>(mut self, collections: I) -> Self
    where
        I: IntoIterator<Item = C>,
        C: Into<String>,
    {
        self.collections = collections.into_iter().map(Into::into).collect();
        self
    }
}

/// A published commit: the complete event plus which changes belong to
/// internal collections. Subscriptions derive their filtered view from it.
#[derive(Debug)]
pub(crate) struct CommittedChange {
    event: ChangeEvent,
    /// Parallel to `event.changes`.
    internal: Vec<bool>,
}

impl CommittedChange {
    /// `changes` pairs each change with whether its collection is internal.
    pub(crate) fn new(
        revision: u64,
        source: ChangeSource,
        catalog_version: u64,
        catalog_changed: bool,
        changes: impl IntoIterator<Item = (ChangedEntity, bool)>,
    ) -> Self {
        let (changes, internal) = changes.into_iter().unzip();
        Self {
            event: ChangeEvent {
                revision,
                committed_at: DateTime::now_utc(),
                source,
                changes,
                catalog_version,
                catalog_changed,
            },
            internal,
        }
    }

    /// The event as seen by a subscription, or `None` when the subscription
    /// has nothing to see in this commit.
    fn view(&self, filter: &Filter) -> Option<ChangeEvent> {
        let changes = self
            .event
            .changes
            .iter()
            .zip(&self.internal)
            .filter(|(change, internal)| {
                (filter.internal || !**internal)
                    && filter
                        .collections
                        .as_ref()
                        .is_none_or(|collections| collections.contains(&change.collection))
            })
            .map(|(change, _)| {
                if filter.payloads {
                    change.clone()
                } else {
                    change.without_payloads()
                }
            })
            .collect::<Vec<_>>();
        let catalog_visible = self.event.catalog_changed && filter.collections.is_none();
        if changes.is_empty() && !catalog_visible {
            return None;
        }
        Some(ChangeEvent {
            revision: self.event.revision,
            committed_at: self.event.committed_at,
            source: self.event.source,
            changes,
            catalog_version: self.event.catalog_version,
            catalog_changed: self.event.catalog_changed,
        })
    }
}

#[derive(Debug)]
struct Filter {
    payloads: bool,
    internal: bool,
    collections: Option<BTreeSet<String>>,
}

impl From<ChangeSubscriptionOptions> for Filter {
    fn from(options: ChangeSubscriptionOptions) -> Self {
        Self {
            payloads: options.include_payloads,
            internal: options.include_internal,
            collections: (!options.collections.is_empty())
                .then(|| options.collections.into_iter().collect()),
        }
    }
}

/// Publisher side of the change feed. Cheap to clone; clones share their
/// subscribers. Subscriptions end when every clone is dropped.
#[derive(Clone)]
pub struct ChangeFeed {
    sender: Arc<channel::Sender>,
}

impl ChangeFeed {
    /// A feed retaining up to `capacity` undelivered events per subscriber.
    pub fn new(capacity: usize) -> Self {
        Self {
            sender: Arc::new(channel::Sender::new(capacity.max(1))),
        }
    }

    pub fn subscribe(&self, options: ChangeSubscriptionOptions) -> ChangeSubscription {
        ChangeSubscription {
            inner: self.sender.subscribe(),
            filter: options.into(),
            skipped: 0,
            last_revision: None,
            pending: None,
        }
    }

    /// Number of live subscriptions.
    pub fn subscriber_count(&self) -> usize {
        self.sender.receiver_count()
    }

    pub(crate) fn publish(&self, change: CommittedChange) {
        self.sender.send(Arc::new(change));
    }
}

impl Default for ChangeFeed {
    fn default() -> Self {
        Self::new(DEFAULT_CHANGE_FEED_CAPACITY)
    }
}

impl std::fmt::Debug for ChangeFeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChangeFeed")
            .field("subscribers", &self.subscriber_count())
            .finish()
    }
}

/// A live subscription to a [`ChangeFeed`]. Dropping it stops delivery.
pub struct ChangeSubscription {
    inner: BoxStream<'static, channel::Received>,
    filter: Filter,
    /// Commits dropped since the last delivered item.
    skipped: u64,
    last_revision: Option<u64>,
    /// The first retained commit after a lag, delivered after the notice.
    pending: Option<Arc<CommittedChange>>,
}

impl ChangeSubscription {
    fn lagged(&mut self, resume_revision: u64) -> ChangeFeedItem {
        ChangeFeedItem::Lagged {
            skipped: std::mem::take(&mut self.skipped),
            resume_revision,
        }
    }
}

impl Stream for ChangeSubscription {
    type Item = ChangeFeedItem;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            let change = match this.pending.take() {
                Some(change) => change,
                None => match ready!(this.inner.poll_next_unpin(cx)) {
                    Some(channel::Received::Change(change)) => change,
                    Some(channel::Received::Lagged(skipped)) => {
                        this.skipped += skipped;
                        continue;
                    }
                    None if this.skipped > 0 => {
                        let resume = this.last_revision.map_or(0, |revision| revision + 1);
                        return Poll::Ready(Some(this.lagged(resume)));
                    }
                    None => return Poll::Ready(None),
                },
            };
            if this.skipped > 0 {
                let resume = change.event.revision;
                this.pending = Some(change);
                return Poll::Ready(Some(this.lagged(resume)));
            }
            this.last_revision = Some(change.event.revision);
            if let Some(event) = change.view(&this.filter) {
                return Poll::Ready(Some(ChangeFeedItem::Event(event)));
            }
        }
    }
}

impl std::fmt::Debug for ChangeSubscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChangeSubscription")
            .field("filter", &self.filter)
            .finish_non_exhaustive()
    }
}

/// Bounded broadcast over `tokio::sync::broadcast`.
#[cfg(feature = "tokio")]
mod channel {
    use std::sync::Arc;

    use futures::StreamExt as _;
    use futures::stream::BoxStream;
    use tokio::sync::broadcast::{self, error::RecvError};

    use super::CommittedChange;

    pub(super) enum Received {
        Change(Arc<CommittedChange>),
        /// This many changes were dropped before the next one.
        Lagged(u64),
    }

    pub(super) struct Sender(broadcast::Sender<Arc<CommittedChange>>);

    impl Sender {
        pub(super) fn new(capacity: usize) -> Self {
            Self(broadcast::channel(capacity).0)
        }

        pub(super) fn receiver_count(&self) -> usize {
            self.0.receiver_count()
        }

        pub(super) fn send(&self, change: Arc<CommittedChange>) {
            // Fails only without receivers, which is not an error.
            let _ = self.0.send(change);
        }

        pub(super) fn subscribe(&self) -> BoxStream<'static, Received> {
            futures::stream::unfold(self.0.subscribe(), |mut receiver| async move {
                let received = match receiver.recv().await {
                    Ok(change) => Received::Change(change),
                    Err(RecvError::Lagged(skipped)) => Received::Lagged(skipped),
                    Err(RecvError::Closed) => return None,
                };
                Some((received, receiver))
            })
            .boxed()
        }
    }
}

/// Bounded broadcast without an async runtime: one queue per subscriber
/// that drops its oldest entry when full.
#[cfg(not(feature = "tokio"))]
mod channel {
    use std::collections::VecDeque;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
    use std::task::{Context, Poll, Waker};

    use futures::Stream;
    use futures::stream::BoxStream;

    use super::CommittedChange;

    pub(super) enum Received {
        Change(Arc<CommittedChange>),
        /// This many changes were dropped before the next one.
        Lagged(u64),
    }

    pub(super) struct Sender {
        capacity: usize,
        queues: Mutex<Vec<Weak<Queue>>>,
    }

    impl Sender {
        pub(super) fn new(capacity: usize) -> Self {
            Self {
                capacity,
                queues: Mutex::new(Vec::new()),
            }
        }

        fn queues(&self) -> MutexGuard<'_, Vec<Weak<Queue>>> {
            self.queues.lock().unwrap_or_else(PoisonError::into_inner)
        }

        pub(super) fn receiver_count(&self) -> usize {
            self.queues()
                .iter()
                .filter(|queue| queue.strong_count() > 0)
                .count()
        }

        pub(super) fn send(&self, change: Arc<CommittedChange>) {
            self.queues().retain(|queue| match queue.upgrade() {
                Some(queue) => {
                    queue.push(Arc::clone(&change), self.capacity);
                    true
                }
                None => false,
            });
        }

        pub(super) fn subscribe(&self) -> BoxStream<'static, Received> {
            let queue = Arc::new(Queue::default());
            self.queues().push(Arc::downgrade(&queue));
            Box::pin(Receiver(queue))
        }
    }

    impl Drop for Sender {
        fn drop(&mut self) {
            for queue in self.queues().drain(..).filter_map(|queue| queue.upgrade()) {
                let mut state = queue.state();
                state.closed = true;
                if let Some(waker) = state.waker.take() {
                    waker.wake();
                }
            }
        }
    }

    #[derive(Default)]
    struct Queue(Mutex<QueueState>);

    #[derive(Default)]
    struct QueueState {
        changes: VecDeque<Arc<CommittedChange>>,
        skipped: u64,
        closed: bool,
        waker: Option<Waker>,
    }

    impl Queue {
        fn state(&self) -> MutexGuard<'_, QueueState> {
            self.0.lock().unwrap_or_else(PoisonError::into_inner)
        }

        fn push(&self, change: Arc<CommittedChange>, capacity: usize) {
            let mut state = self.state();
            if state.changes.len() >= capacity {
                state.changes.pop_front();
                state.skipped += 1;
            }
            state.changes.push_back(change);
            if let Some(waker) = state.waker.take() {
                waker.wake();
            }
        }
    }

    struct Receiver(Arc<Queue>);

    impl Stream for Receiver {
        type Item = Received;

        fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Received>> {
            let mut state = self.0.state();
            if state.skipped > 0 {
                return Poll::Ready(Some(Received::Lagged(std::mem::take(&mut state.skipped))));
            }
            if let Some(change) = state.changes.pop_front() {
                return Poll::Ready(Some(Received::Change(change)));
            }
            if state.closed {
                return Poll::Ready(None);
            }
            state.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

#[cfg(test)]
mod tests {
    use futures::FutureExt as _;
    use semantic_data::value::Value;

    use super::*;

    fn object(name: &str) -> Object {
        Object::from_iter([("name".to_string(), Value::String(name.to_string()))])
    }

    fn commit(revision: u64, changes: Vec<(&str, &str, bool)>) -> CommittedChange {
        CommittedChange::new(
            revision,
            ChangeSource::Batch,
            1,
            false,
            changes.into_iter().map(|(collection, id, internal)| {
                (
                    ChangedEntity::new(collection.into(), id.into(), None, Some(object(id)))
                        .unwrap(),
                    internal,
                )
            }),
        )
    }

    /// The next item if one is ready.
    fn next(subscription: &mut ChangeSubscription) -> Option<ChangeFeedItem> {
        subscription.next().now_or_never().flatten()
    }

    fn event(item: Option<ChangeFeedItem>) -> ChangeEvent {
        match item {
            Some(ChangeFeedItem::Event(event)) => event,
            other => panic!("expected an event, got {other:?}"),
        }
    }

    #[test]
    fn filters_internal_collections_and_payloads() {
        let feed = ChangeFeed::default();
        let mut default = feed.subscribe(ChangeSubscriptionOptions::default());
        let mut full = feed.subscribe(
            ChangeSubscriptionOptions::default()
                .with_payloads()
                .with_internal(),
        );
        let mut notes =
            feed.subscribe(ChangeSubscriptionOptions::default().with_collections(["notes"]));
        feed.publish(commit(
            1,
            vec![("__internal", "a", true), ("notes", "n", false)],
        ));
        feed.publish(commit(2, vec![("__internal", "b", true)]));
        feed.publish(commit(3, vec![("tasks", "t", false)]));

        let first = event(next(&mut default));
        assert_eq!(first.revision, 1);
        assert_eq!(first.changes.len(), 1);
        assert_eq!(first.changes[0].id, "n");
        assert_eq!(first.changes[0].kind, ChangeKind::Created);
        assert_eq!(first.changes[0].after, None);
        // Internal-only commits are invisible to the default subscription.
        assert_eq!(event(next(&mut default)).revision, 3);
        assert_eq!(next(&mut default), None);

        let first = event(next(&mut full));
        assert_eq!(first.changes.len(), 2);
        assert_eq!(first.changes[1].after, Some(object("n")));
        assert_eq!(event(next(&mut full)).revision, 2);
        assert_eq!(event(next(&mut full)).revision, 3);

        assert_eq!(event(next(&mut notes)).revision, 1);
        assert_eq!(next(&mut notes), None);
    }

    #[test]
    fn catalog_changes_reach_unfiltered_subscriptions_only() {
        let feed = ChangeFeed::default();
        let mut all = feed.subscribe(ChangeSubscriptionOptions::default());
        let mut notes =
            feed.subscribe(ChangeSubscriptionOptions::default().with_collections(["notes"]));
        feed.publish(CommittedChange::new(
            1,
            ChangeSource::Ddl,
            2,
            true,
            std::iter::empty(),
        ));
        let event = event(next(&mut all));
        assert!(event.catalog_changed);
        assert_eq!(event.catalog_version, 2);
        assert_eq!(next(&mut notes), None);
    }

    #[test]
    fn lagging_subscriber_is_told_where_delivery_resumes() {
        let feed = ChangeFeed::new(2);
        let mut slow = feed.subscribe(ChangeSubscriptionOptions::default());
        for revision in 1..=5 {
            feed.publish(commit(revision, vec![("notes", "n", false)]));
        }
        assert_eq!(
            next(&mut slow),
            Some(ChangeFeedItem::Lagged {
                skipped: 3,
                resume_revision: 4
            })
        );
        assert_eq!(event(next(&mut slow)).revision, 4);
        assert_eq!(event(next(&mut slow)).revision, 5);
        assert_eq!(next(&mut slow), None);
    }

    #[test]
    fn dropping_subscriptions_and_feed() {
        let feed = ChangeFeed::default();
        let first = feed.subscribe(ChangeSubscriptionOptions::default());
        let mut second = feed.subscribe(ChangeSubscriptionOptions::default());
        assert_eq!(feed.subscriber_count(), 2);
        drop(first);
        assert_eq!(feed.subscriber_count(), 1);
        feed.publish(commit(1, vec![("notes", "n", false)]));
        drop(feed);
        assert_eq!(event(next(&mut second)).revision, 1);
        assert_eq!(second.next().now_or_never(), Some(None));
    }

    #[test]
    fn change_kinds() {
        let kind = |before: Option<Object>, after: Option<Object>| {
            ChangedEntity::new("c".into(), "i".into(), before, after).map(|change| change.kind)
        };
        assert_eq!(kind(None, None), None);
        assert_eq!(kind(None, Some(object("a"))), Some(ChangeKind::Created));
        assert_eq!(
            kind(Some(object("a")), Some(object("b"))),
            Some(ChangeKind::Updated)
        );
        assert_eq!(kind(Some(object("a")), None), Some(ChangeKind::Deleted));
    }
}
