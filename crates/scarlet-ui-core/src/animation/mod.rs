//! Declarative animation transactions and interpolation.
//!
//! Model state changes immediately. Supported render properties interpolate
//! their presentation values without rebuilding application Views every frame.

pub(crate) mod pending;
mod runtime;
mod timing;

pub(crate) use runtime::{VisualAnimations, set_active};
pub(crate) use timing::AnimatedValue as Animated;
pub use timing::{Animatable, AnimatedValue, Animation};

/// Animation policy carried from a state mutation into its deferred rebuild.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Transaction {
    /// Animation requested by this scope, before applying the disable policy.
    pub animation: Option<Animation>,
    /// Prevent descendant `.animation(...)` modifiers from adding an animation.
    pub disables_animations: bool,
}

impl Transaction {
    /// Read the animation policy of the current synchronous scope or rebuild.
    ///
    /// Custom render-object updaters can pass
    /// `Transaction::current().effective_animation()` to
    /// [`AnimatedValue::retarget`] to honor inherited animation modifiers and
    /// [`without_animation`]. Outside a scope, this returns the default policy.
    /// Policies do not cross detached task or thread boundaries.
    pub fn current() -> Self {
        current_transaction()
    }

    /// Return the requested animation unless this transaction disables it.
    pub fn effective_animation(self) -> Option<Animation> {
        if self.disables_animations {
            None
        } else {
            self.animation
        }
    }
}

#[cfg(feature = "std")]
std::thread_local! {
    static CURRENT: core::cell::Cell<Transaction> = const {
        core::cell::Cell::new(Transaction { animation: None, disables_animations: false })
    };
}

// The legacy runtime has no typed TLS. Getpid identifies the calling task in
// that runtime, so detached workers cannot inherit the UI task's transaction.
#[cfg(not(feature = "std"))]
static CURRENT: crate::os::Mutex<alloc::collections::BTreeMap<u32, Transaction>> =
    crate::os::Mutex::new(alloc::collections::BTreeMap::new());

pub(crate) fn current_transaction() -> Transaction {
    #[cfg(feature = "std")]
    {
        CURRENT.with(|current| current.get())
    }
    #[cfg(not(feature = "std"))]
    {
        CURRENT
            .lock()
            .get(&std::task::getpid())
            .copied()
            .unwrap_or_default()
    }
}

fn replace_transaction(transaction: Transaction) -> Transaction {
    #[cfg(feature = "std")]
    {
        CURRENT.with(|current| current.replace(transaction))
    }
    #[cfg(not(feature = "std"))]
    {
        let mut current = CURRENT.lock();
        let task = std::task::getpid();
        if transaction == Transaction::default() {
            current.remove(&task).unwrap_or_default()
        } else {
            current.insert(task, transaction).unwrap_or_default()
        }
    }
}

/// Execute synchronous state changes under an animation policy.
///
/// Nested scopes restore the previous policy, including when unwinding. The
/// scope does not cross an await, detached task or thread boundary.
pub fn with_transaction<R>(transaction: Transaction, body: impl FnOnce() -> R) -> R {
    if transaction == current_transaction() {
        return body();
    }
    struct Restore(Transaction);
    impl Drop for Restore {
        fn drop(&mut self) {
            replace_transaction(self.0);
        }
    }
    let _restore = Restore(replace_transaction(transaction));
    body()
}

/// Animate supported properties changed by the enclosed synchronous mutation.
/// Pass `None` to remove the inherited animation. Use `without_animation` to
/// also suppress descendant animation modifiers.
pub fn with_animation<R>(animation: impl Into<Option<Animation>>, body: impl FnOnce() -> R) -> R {
    let mut transaction = current_transaction();
    transaction.animation = animation.into();
    with_transaction(transaction, body)
}

/// Apply changes immediately, even inside a View's animation scope.
pub fn without_animation<R>(body: impl FnOnce() -> R) -> R {
    with_transaction(
        Transaction {
            animation: None,
            disables_animations: true,
        },
        body,
    )
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod public_api_tests {
    use super::*;
    use core::time::Duration;

    #[test]
    fn current_transaction_scopes_control_public_animated_values() {
        const REPEAT: Animation = Animation::linear(Duration::from_secs(1)).repeat_forever(false);
        let mut value: crate::AnimatedValue<f32> = crate::prelude::AnimatedValue::new(0.0);
        assert_eq!(Transaction::current(), Transaction::default());
        with_animation(REPEAT, || {
            let inherited = Transaction::current();
            assert_eq!(inherited.effective_animation(), Some(REPEAT));
            value.retarget(8.0, inherited.effective_animation());
            value.advance(Duration::from_millis(250));
            assert_eq!(value.value(), 2.0);
            assert!(value.is_active());

            without_animation(|| {
                with_animation(REPEAT, || {
                    let disabled = Transaction::current();
                    assert_eq!(disabled.animation, Some(REPEAT));
                    assert!(disabled.disables_animations);
                    assert_eq!(disabled.effective_animation(), None);
                    value.retarget(4.0, disabled.effective_animation());
                    assert_eq!(value.value(), 4.0);
                    assert!(!value.is_active());
                });
            });
            assert_eq!(Transaction::current(), inherited);
            with_animation(None, || {
                assert_eq!(Transaction::current().effective_animation(), None);
                value.retarget(6.0, Transaction::current().effective_animation());
                assert_eq!(value.value(), 6.0);
                assert!(!value.is_active());
            });
            assert_eq!(Transaction::current(), inherited);
            value.retarget(8.0, Transaction::current().effective_animation());
            assert!(value.is_active());
        });
        assert_eq!(Transaction::current(), Transaction::default());
        // Leaving the transaction scope must not stop the value's own timeline.
        value.advance(Duration::from_millis(1250));
        assert_eq!(value.value(), 6.5);
        assert!(value.is_active());
    }
}
