//! Bounded image-byte caching and in-flight request sharing, independent of HTTP backends.
use futures::{
    FutureExt,
    future::{BoxFuture, Shared},
};
use std::{
    collections::{HashMap, VecDeque},
    hash::Hash,
    sync::{Arc, Mutex},
};

/// Implement on an immutable response or transformed-image value.
pub trait ImageCacheValue: Send + Sync + 'static {
    fn byte_len(&self) -> usize;
    /// Failed responses should return false so the next request can retry.
    fn cacheable(&self) -> bool {
        true
    }
}
type Download<V> = Shared<BoxFuture<'static, Result<Arc<V>, Arc<String>>>>;
struct Pending<V> {
    download: Download<V>,
    waiters: usize,
    generation: u64,
}
struct State<K, V> {
    entries: VecDeque<(K, Arc<V>)>,
    pending: HashMap<K, Pending<V>>,
    generation: u64,
    bytes: usize,
}
impl<K: Eq, V: ImageCacheValue> State<K, V> {
    fn get(&mut self, key: &K) -> Option<Arc<V>> {
        let index = self.entries.iter().position(|(known, _)| known == key)?;
        let entry = self.entries.remove(index)?;
        let value = entry.1.clone();
        self.entries.push_back(entry);
        Some(value)
    }
    fn insert(&mut self, key: K, value: Arc<V>, budget: usize, capacity: usize) {
        if capacity == 0
            || !value.cacheable()
            || value.byte_len() > budget
            || self.get(&key).is_some()
        {
            return;
        }
        while self.bytes + value.byte_len() > budget || self.entries.len() >= capacity {
            let Some((_, removed)) = self.entries.pop_front() else {
                break;
            };
            self.bytes -= removed.byte_len();
        }
        self.bytes += value.byte_len();
        self.entries.push_back((key, value));
    }
}
/// Keys belong to the host: include an identity namespace and transform fingerprint
/// when needed. Credentials, HTTP requests, disk storage and image transforms stay
/// outside this cache. Completed values are bounded by both bytes and entry count.
pub struct ImageByteCache<K, V> {
    state: Mutex<State<K, V>>,
    budget: usize,
    capacity: usize,
}
// The final cancelled waiter removes its shared download, releasing any loader
// state. A request that was never polled never creates a pending entry.
struct Waiter<K: Eq + Hash, V> {
    cache: Arc<ImageByteCache<K, V>>,
    key: K,
    generation: u64,
}
impl<K: Eq + Hash, V> Drop for Waiter<K, V> {
    fn drop(&mut self) {
        let mut state = self.cache.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(pending) = state.pending.get_mut(&self.key) {
            if pending.generation == self.generation {
                pending.waiters -= 1;
                if pending.waiters == 0 {
                    state.pending.remove(&self.key);
                }
            }
        }
    }
}
impl<K, V> Default for ImageByteCache<K, V> {
    fn default() -> Self {
        Self::with_limits(64 * 1024 * 1024, 256)
    }
}
impl<K, V> ImageByteCache<K, V> {
    pub fn with_limits(budget: usize, capacity: usize) -> Self {
        Self {
            state: Mutex::new(State {
                entries: VecDeque::new(),
                pending: HashMap::new(),
                generation: 0,
                bytes: 0,
            }),
            budget,
            capacity,
        }
    }
    pub fn len(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entries
            .len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn byte_len(&self) -> usize {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).bytes
    }
}
impl<K: Eq + Hash + Clone + Send + 'static, V: ImageCacheValue> ImageByteCache<K, V> {
    pub fn get(&self, key: &K) -> Option<Arc<V>> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
    }
    pub fn insert(&self, key: K, value: Arc<V>) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).insert(
            key,
            value,
            self.budget,
            self.capacity,
        );
    }
    /// The loader runs only on a miss. Concurrent callers with the same key share
    /// its result. Merely constructing and dropping this future starts no work.
    pub fn get_or_load<F, Fut>(
        self: &Arc<Self>,
        key: K,
        loader: F,
    ) -> BoxFuture<'static, Result<Arc<V>, Arc<String>>>
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<V, String>> + Send + 'static,
    {
        let cache = self.clone();
        async move {
            let (download, generation) = {
                let mut state = cache.state.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(value) = state.get(&key) {
                    return Ok(value);
                }
                if let Some(pending) = state.pending.get_mut(&key) {
                    pending.waiters += 1;
                    (pending.download.clone(), pending.generation)
                } else {
                    let download = async move { loader().await.map(Arc::new).map_err(Arc::new) }
                        .boxed()
                        .shared();
                    state.generation = state.generation.wrapping_add(1);
                    let generation = state.generation;
                    state.pending.insert(
                        key.clone(),
                        Pending {
                            download: download.clone(),
                            waiters: 1,
                            generation,
                        },
                    );
                    (download, generation)
                }
            };
            let _waiter = Waiter {
                cache: cache.clone(),
                key: key.clone(),
                generation,
            };
            let result = download.await;
            let mut state = cache.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Ok(value) = &result {
                state.insert(key, value.clone(), cache.budget, cache.capacity);
            }
            result
        }
        .boxed()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Bytes(Vec<u8>);
    impl ImageCacheValue for Bytes {
        fn byte_len(&self) -> usize {
            self.0.len()
        }
    }
    #[test]
    fn cancelling_the_final_waiter_releases_the_pending_loader() {
        let cache = Arc::new(ImageByteCache::default());
        let mut request = cache.get_or_load("image", || {
            futures::future::pending::<Result<Bytes, String>>()
        });
        let mut context = std::task::Context::from_waker(futures::task::noop_waker_ref());
        assert!(request.as_mut().poll(&mut context).is_pending());
        assert_eq!(cache.state.lock().unwrap().pending.len(), 1);
        drop(request);
        assert!(cache.state.lock().unwrap().pending.is_empty());
        let result = futures::executor::block_on(
            cache.get_or_load("image", || async { Ok(Bytes(vec![2])) }),
        )
        .unwrap();
        assert_eq!(result.0, vec![2]);
    }

    #[test]
    fn pending_callers_share_a_single_loader() {
        let cache = Arc::new(ImageByteCache::default());
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let first_calls = calls.clone();
        let second_calls = calls.clone();
        let (send, receive) = futures::channel::oneshot::channel();
        let mut first = cache.get_or_load("same", move || async move {
            first_calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            receive.await.unwrap();
            Ok(Bytes(vec![1]))
        });
        let mut second = cache.get_or_load("same", move || async move {
            second_calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(Bytes(vec![2]))
        });
        let mut context = std::task::Context::from_waker(futures::task::noop_waker_ref());
        assert!(first.as_mut().poll(&mut context).is_pending());
        assert!(second.as_mut().poll(&mut context).is_pending());
        send.send(()).unwrap();
        let (first, second) = futures::executor::block_on(futures::future::join(first, second));
        assert!(Arc::ptr_eq(&first.unwrap(), &second.unwrap()));
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
    }

    #[test]
    fn evicts_by_bytes_and_handles_disabled_cache() {
        let cache = ImageByteCache::with_limits(5, 10);
        cache.insert("a", Arc::new(Bytes(vec![0; 3])));
        cache.insert("b", Arc::new(Bytes(vec![0; 3])));
        assert!(cache.get(&"a").is_none());
        assert_eq!(cache.byte_len(), 3);
        let disabled = ImageByteCache::with_limits(5, 0);
        disabled.insert("a", Arc::new(Bytes(vec![])));
        assert!(disabled.is_empty());
    }
    #[test]
    fn namespaces_are_isolated_and_failed_loads_can_retry() {
        let cache = Arc::new(ImageByteCache::default());
        futures::executor::block_on(async {
            assert!(
                cache
                    .get_or_load(("user-a", "image"), || async {
                        Err::<Bytes, _>("offline".into())
                    })
                    .await
                    .is_err()
            );
            cache
                .get_or_load(("user-a", "image"), || async { Ok(Bytes(vec![1])) })
                .await
                .unwrap();
            assert!(cache.get(&("user-b", "image")).is_none());
            assert_eq!(cache.get(&("user-a", "image")).unwrap().0, vec![1]);
        });
    }
}
