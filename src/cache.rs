use std::{fmt::Debug, hash::Hash, sync::Arc};

use dashmap::DashMap;
use futures::{
    future::{BoxFuture, Shared},
    Future, FutureExt,
};

use crate::progress::PROGRESS_BAR;

type SharedBoxFuture<T> = Shared<BoxFuture<'static, T>>;

pub struct Cache<K: Eq + Hash + Clone + Send + Debug + 'static, V: Clone + Send + 'static> {
    loader: Box<dyn Fn(K) -> BoxFuture<'static, V> + Send + Sync + 'static>,
    map: DashMap<K, SharedBoxFuture<V>>,
}

impl<K: Eq + Hash + Clone + Send + Debug + 'static, V: Clone + Send + 'static> Cache<K, V> {
    pub fn new<T, F>(loader: T) -> Self
    where
        F: Future<Output = V> + Sized + Send + 'static,
        T: Fn(K) -> F + Send + Sync + Clone + 'static,
    {
        let loader = Arc::new(loader);

        Self {
            loader: Box::new({
                move |key| {
                    let loader = loader.clone();
                    Box::pin({
                        async move {
                            PROGRESS_BAR.inc_length(1);
                            let v = loader(key).await;
                            PROGRESS_BAR.inc(1);
                            v
                        }
                    })
                }
            }),
            map: DashMap::new(),
        }
    }

    pub async fn get(&self, key: K) -> V {
        let f = self
            .map
            .entry(key.clone())
            .or_insert_with(|| (self.loader)(key).boxed().shared())
            .clone();

        f.await
    }
}
