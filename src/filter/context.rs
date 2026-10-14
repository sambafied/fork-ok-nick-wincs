//! Opaque callback-context tokens and active-callback accounting.
//!
//! `CfDisconnectSyncRoot` does not wait for callbacks that are already
//! dispatched (qualified by the `disconnect_during_inflight_callbacks`
//! native test). A heap pointer passed as `CallbackContext` could therefore be
//! read after it was freed. Instead the context value is a non-zero integer key
//! into a process-wide registry of `Weak` filter references. The value is never
//! dereferenced; removing a registration is always safe, and late callbacks
//! simply fail to resolve a filter.
//!
//! Each resolved callback holds an [Active] guard. Resolution increments the
//! counter while holding the registry read lock, and removal takes the write
//! lock, so once a token is removed no new callback can become active: an
//! observed count of zero after removal is final quiescence.
use std::{
    any::Any,
    collections::HashMap,
    ffi::c_void,
    ops::Deref,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, OnceLock, RwLock, Weak,
    },
};

/// One registered filter: its weak reference and shared active-callback count.
struct Entry {
    filter: Box<dyn Any + Send + Sync>,
    active: Arc<AtomicUsize>,
}

/// Registered entries keyed by token value.
type Registry = RwLock<HashMap<usize, Entry>>;

/// Next token value; zero is never issued so a null context never resolves.
static NEXT: AtomicUsize = AtomicUsize::new(1);
static REGISTRY: OnceLock<Registry> = OnceLock::new();

fn registry() -> &'static Registry {
    REGISTRY.get_or_init(Registry::default)
}

/// Owns one registry entry. Dropping the token removes the entry, after which
/// callbacks carrying its value resolve to `None`.
#[derive(Debug)]
pub struct ContextToken {
    key: usize,
    active: Arc<AtomicUsize>,
}

impl ContextToken {
    /// Registers a weak reference to `filter` and returns its token.
    pub fn register<T: Send + Sync + 'static>(filter: &Arc<T>) -> Self {
        let key = NEXT.fetch_add(1, Ordering::Relaxed);
        let active = Arc::new(AtomicUsize::new(0));
        let entry = Entry {
            filter: Box::new(Arc::downgrade(filter)),
            active: active.clone(),
        };
        // A poisoned lock only means another thread panicked mid-update; the
        // map itself remains structurally valid.
        registry()
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(key, entry);
        Self { key, active }
    }

    /// Opaque value for `CallbackContext`; never a dereferenceable pointer.
    pub fn as_context(&self) -> *const c_void {
        self.key as *const c_void
    }

    /// Shared counter of callbacks currently executing filter code. Remains
    /// valid after the token is dropped.
    pub fn active(&self) -> Arc<AtomicUsize> {
        self.active.clone()
    }
}

impl Drop for ContextToken {
    fn drop(&mut self) {
        registry()
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&self.key);
    }
}

/// A live filter reference held for the duration of one callback.
pub struct Active<T> {
    filter: Arc<T>,
    active: Arc<AtomicUsize>,
}

impl<T> Deref for Active<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.filter
    }
}

impl<T> Drop for Active<T> {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Resolves a callback context value to a live filter, if still registered.
pub fn resolve<T: Send + Sync + 'static>(context: *const c_void) -> Option<Active<T>> {
    let key = context as usize;
    if key == 0 {
        return None;
    }
    let registry = registry()
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let entry = registry.get(&key)?;
    let filter = entry.filter.downcast_ref::<Weak<T>>()?.upgrade()?;
    // Incremented under the read lock; see module documentation.
    entry.active.fetch_add(1, Ordering::AcqRel);
    Some(Active {
        filter,
        active: entry.active.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_resolves_until_dropped_and_never_after() {
        let filter = Arc::new(7_u32);
        let token = ContextToken::register(&filter);
        let value = token.as_context();
        assert_eq!(resolve::<u32>(value).as_deref(), Some(&7));
        assert!(
            resolve::<u64>(value).is_none(),
            "wrong type must not resolve"
        );
        drop(token);
        assert!(resolve::<u32>(value).is_none(), "removed token resolved");
        assert!(resolve::<u32>(std::ptr::null()).is_none());
    }

    #[test]
    fn dropped_filter_does_not_resolve() {
        let filter = Arc::new(String::from("filter"));
        let token = ContextToken::register(&filter);
        drop(filter);
        assert!(resolve::<String>(token.as_context()).is_none());
    }

    #[test]
    fn active_count_tracks_guards_beyond_token_removal() {
        let filter = Arc::new(1_u8);
        let token = ContextToken::register(&filter);
        let active = token.active();
        let first = resolve::<u8>(token.as_context()).expect("registered");
        let second = resolve::<u8>(token.as_context()).expect("registered");
        assert_eq!(active.load(Ordering::Acquire), 2);
        drop(token);
        assert!(resolve::<u8>(std::ptr::null()).is_none());
        drop(first);
        assert_eq!(active.load(Ordering::Acquire), 1, "guard outlives token");
        drop(second);
        assert_eq!(active.load(Ordering::Acquire), 0);
    }
}
