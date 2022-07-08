use std::{collections::HashMap, fmt::Debug, hash::Hash, sync::Arc};

use futures::{
    future::{BoxFuture, Shared},
    Future, FutureExt,
};
use generational_arena::Arena;
use tokio::sync::Mutex;

type SharedBoxFuture<T> = Shared<BoxFuture<'static, T>>;

pub struct Cache<
    K: Eq + Hash + Clone + Send + Debug + 'static,
    V: Clone + Send + 'static,
    M: Send + Clone + 'static = (),
> {
    loader: Box<dyn Fn(K, M) -> BoxFuture<'static, V> + Send + Sync + 'static>,
    map: Mutex<HashMap<K, (SharedBoxFuture<V>, M)>>,
}

impl<
        K: Eq + Hash + Clone + Send + Debug + 'static,
        V: Clone + Send + 'static,
        M: Send + Clone + 'static,
    > Cache<K, V, M>
{
    pub fn new<T, F>(loader: T) -> Self
    where
        F: Future<Output = V> + Sized + Send + 'static,
        T: Fn(K, M) -> F + Send + Sync + Clone + 'static,
    {
        let loader = Arc::new(loader);
        let arena = Arc::new(Mutex::new(Arena::new()));

        Self {
            loader: Box::new({
                move |key, meta| {
                    let loader = loader.clone();
                    Box::pin({
                        let arena = arena.clone();
                        async move {
                            let i = arena.lock().await.insert(key.clone());
                            let v = loader(key, meta).await;
                            arena.lock().await.remove(i);
                            v
                        }
                    })
                }
            }),
            map: Mutex::new(HashMap::new()),
        }
    }

    pub async fn get(&self, key: K, meta: M) -> V {
        let mut map = self.map.lock().await;

        let f = map
            .entry(key.clone())
            .or_insert_with(|| ((self.loader)(key, meta.clone()).boxed().shared(), meta))
            .clone();

        drop(map);

        f.0.await
    }

    pub async fn get_meta(&self, key: &K) -> Option<M> {
        let map = self.map.lock().await;
        Some(map.get(key)?.1.clone())
    }
}
