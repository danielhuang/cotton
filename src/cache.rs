use std::{collections::HashMap, hash::Hash, sync::Arc};

use futures::{
    future::{BoxFuture, Shared},
    Future, FutureExt,
};
use tokio::sync::Mutex;

pub struct Cache<
    K: Eq + Hash + Clone + Send + 'static,
    V: Clone + Send + 'static,
    M: Send + Clone + 'static = (),
> {
    loader: Box<dyn Fn(K, M) -> BoxFuture<'static, V> + Send + Sync + 'static>,
    map: Mutex<HashMap<K, Shared<BoxFuture<'static, V>>>>,
}

impl<
        K: Eq + Hash + Clone + Send + 'static,
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

        Self {
            loader: Box::new(move |key, meta| {
                let loader = loader.clone();
                Box::pin(async move { loader(key, meta).await })
            }),
            map: Mutex::new(HashMap::new()),
        }
    }

    pub async fn get(&self, key: K, meta: M) -> V {
        let mut map = self.map.lock().await;

        let f = map
            .entry(key.clone())
            .or_insert_with(|| (self.loader)(key, meta).boxed().shared())
            .clone();

        drop(map);

        f.await
    }
}
