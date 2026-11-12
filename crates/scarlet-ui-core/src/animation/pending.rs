//! Transactions belonging to independently dirty descendants in one build flush.
//!
//! Reconciliation can update a child before the build queue reaches it. Keep a
//! scoped snapshot so that first update uses the child's own queued policy.
//! The pipeline key prevents an unrelated window from reading this snapshot;
//! nested flushes temporarily replace it and restore it on return or unwind.

use super::Transaction;
use crate::element::ElementId;
use crate::pipeline::PipelineId;
use alloc::collections::{BTreeMap, BTreeSet};

struct PendingBuilds {
    owner: PipelineId,
    dirty: BTreeSet<ElementId>,
    transactions: BTreeMap<ElementId, Transaction>,
    // This is the queued policy of the current rebuild boundary, not the
    // ambient transaction, which local .animation modifiers may override.
    base: Transaction,
}

#[cfg(feature = "std")]
std::thread_local! {
    static CURRENT: core::cell::RefCell<Option<PendingBuilds>> = const {
        core::cell::RefCell::new(None)
    };
}

#[cfg(not(feature = "std"))]
static CURRENT: crate::os::Mutex<BTreeMap<u32, PendingBuilds>> =
    crate::os::Mutex::new(BTreeMap::new());
#[cfg(not(feature = "std"))]
static HAS_PENDING: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

fn replace_pending(pending: Option<PendingBuilds>) -> Option<PendingBuilds> {
    #[cfg(feature = "std")]
    {
        CURRENT.with(|current| current.replace(pending))
    }
    #[cfg(not(feature = "std"))]
    {
        if pending.is_none() && !HAS_PENDING.load(core::sync::atomic::Ordering::Relaxed) {
            return None;
        }
        let task = std::task::getpid();
        let mut current = CURRENT.lock();
        let previous = match pending {
            Some(pending) => current.insert(task, pending),
            None => current.remove(&task),
        };
        HAS_PENDING.store(!current.is_empty(), core::sync::atomic::Ordering::Relaxed);
        previous
    }
}

// Only inspect/copy metadata in this callback. Never invoke Element or View
// code while a RefCell borrow or the legacy task-map mutex is held.
fn with_pending<R>(inspect: impl FnOnce(Option<&mut PendingBuilds>) -> R) -> R {
    #[cfg(feature = "std")]
    {
        CURRENT.with(|current| inspect(current.borrow_mut().as_mut()))
    }
    #[cfg(not(feature = "std"))]
    {
        if !HAS_PENDING.load(core::sync::atomic::Ordering::Relaxed) {
            return inspect(None);
        }
        inspect(CURRENT.lock().get_mut(&std::task::getpid()))
    }
}

/// Make an animated build batch visible to retained-child reconciliation.
///
/// An all-default batch needs no snapshot or element-map lookup. Dirty IDs
/// still matter in mixed batches: an absent transaction for a dirty child is
/// its explicit default policy, whereas a clean child inherits its parent.
pub(crate) fn with_pending_builds<R>(
    owner: PipelineId,
    dirty: &BTreeSet<ElementId>,
    transactions: &BTreeMap<ElementId, Transaction>,
    body: impl FnOnce() -> R,
) -> R {
    let pending = (!transactions.is_empty() && dirty.len() > 1).then(|| PendingBuilds {
        owner,
        dirty: dirty.clone(),
        transactions: transactions.clone(),
        base: Transaction::default(),
    });
    struct Restore(Option<PendingBuilds>);
    impl Drop for Restore {
        fn drop(&mut self) {
            replace_pending(self.0.take());
        }
    }
    // Even an empty nested batch masks the previous snapshot until it returns.
    let _restore = Restore(replace_pending(pending));
    body()
}

fn replace_base(owner: PipelineId, transaction: Transaction) -> Option<Transaction> {
    with_pending(|pending| {
        let pending = pending.filter(|pending| pending.owner == owner)?;
        Some(core::mem::replace(&mut pending.base, transaction))
    })
}

/// Enter a queued policy without holding any metadata borrow across user code.
pub(crate) fn with_element_transaction<R>(
    owner: PipelineId,
    transaction: Transaction,
    body: impl FnOnce() -> R,
) -> R {
    struct Restore {
        owner: PipelineId,
        previous: Option<Transaction>,
    }
    impl Drop for Restore {
        fn drop(&mut self) {
            if let Some(previous) = self.previous {
                replace_base(self.owner, previous);
            }
        }
    }
    let _restore = Restore {
        owner,
        previous: replace_base(owner, transaction),
    };
    super::with_transaction(transaction, body)
}

/// A different queued policy starts a new rebuild boundary before child.update.
///
/// Equal queued policies keep the ambient transaction, including overrides
/// introduced by intervening .animation modifiers. A different policy replaces
/// that inherited context; local modifiers *inside* the new boundary can then
/// override it (unless disables_animations is set).
pub(crate) fn transaction_for_child(owner: PipelineId, id: ElementId) -> Option<Transaction> {
    with_pending(|pending| {
        let pending = pending.filter(|pending| pending.owner == owner)?;
        if !pending.dirty.contains(&id) {
            return None;
        }
        let transaction = pending.transactions.get(&id).copied().unwrap_or_default();
        (transaction != pending.base).then_some(transaction)
    })
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::animation::{Animation, current_transaction, with_transaction};

    #[test]
    fn pending_batches_restore_after_unwind_and_are_window_and_thread_local() {
        let owner = PipelineId::generate();
        let other = PipelineId::generate();
        let parent = ElementId::generate();
        let child = ElementId::generate();
        let dirty = BTreeSet::from([parent, child]);
        let animated = Transaction {
            animation: Some(Animation::default()),
            disables_animations: false,
        };
        let transactions = BTreeMap::from([(parent, animated)]);
        with_pending_builds(owner, &dirty, &transactions, || {
            with_element_transaction(owner, animated, || {
                assert_eq!(
                    transaction_for_child(owner, child),
                    Some(Transaction::default())
                );
                assert_eq!(transaction_for_child(other, child), None);
                std::thread::spawn(move || {
                    assert_eq!(transaction_for_child(owner, child), None);
                })
                .join()
                .unwrap();

                with_pending_builds(other, &dirty, &BTreeMap::from([(child, animated)]), || {
                    with_element_transaction(other, Transaction::default(), || {
                        assert_eq!(transaction_for_child(owner, child), None);
                        assert_eq!(transaction_for_child(other, child), Some(animated));
                    });
                });
                assert_eq!(
                    transaction_for_child(owner, child),
                    Some(Transaction::default())
                );

                let result = std::panic::catch_unwind(|| {
                    // An empty nested batch must mask the outer metadata too.
                    with_pending_builds(owner, &dirty, &BTreeMap::new(), || {
                        assert_eq!(transaction_for_child(owner, child), None);
                        panic!("nested flush");
                    });
                });
                assert!(result.is_err());
                assert_eq!(
                    transaction_for_child(owner, child),
                    Some(Transaction::default())
                );

                let result = std::panic::catch_unwind(|| {
                    with_element_transaction(owner, Transaction::default(), || {
                        assert_eq!(transaction_for_child(owner, parent), Some(animated));
                        panic!("child update");
                    });
                });
                assert!(result.is_err());
                assert_eq!(current_transaction(), animated);
                assert_eq!(
                    transaction_for_child(owner, child),
                    Some(Transaction::default())
                );
            });
        });
        assert_eq!(transaction_for_child(owner, child), None);
        assert_eq!(current_transaction(), Transaction::default());

        let result = std::panic::catch_unwind(|| {
            with_pending_builds(owner, &dirty, &transactions, || {
                with_element_transaction(owner, animated, || panic!("outer flush"));
            });
        });
        assert!(result.is_err());
        assert_eq!(transaction_for_child(owner, child), None);
        assert_eq!(current_transaction(), Transaction::default());
    }

    #[test]
    fn equal_queued_policies_preserve_local_overrides_and_clean_children_inherit() {
        let owner = PipelineId::generate();
        let parent = ElementId::generate();
        let child = ElementId::generate();
        let independent = ElementId::generate();
        let clean = ElementId::generate();
        let animated = Transaction {
            animation: Some(Animation::default()),
            disables_animations: false,
        };
        let local = Transaction {
            animation: None,
            disables_animations: true,
        };
        let dirty = BTreeSet::from([parent, child, independent]);
        let transactions = BTreeMap::from([(parent, animated), (child, animated)]);
        with_pending_builds(owner, &dirty, &transactions, || {
            with_element_transaction(owner, animated, || {
                with_transaction(local, || {
                    // Compare queued bases, not the locally overridden ambient
                    // policy, or this child's explicit animation would erase it.
                    assert_eq!(transaction_for_child(owner, child), None);
                    assert_eq!(transaction_for_child(owner, clean), None);
                    assert_eq!(
                        transaction_for_child(owner, independent),
                        Some(Transaction::default())
                    );
                    assert_eq!(current_transaction(), local);
                });
            });
        });
    }
}
