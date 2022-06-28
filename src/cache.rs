use std::{collections::HashMap, hash::Hash, sync::Arc};

use futures::{
    future::{BoxFuture, Shared},
    Future, FutureExt,
};
use tokio::sync::Mutex;

pub struct Cache<K: Eq + Hash + Clone + Send + 'static, V: Clone + Send + 'static> {
    loader: Box<dyn Fn(K) -> BoxFuture<'static, V> + Send + Sync + 'static>,
    map: Mutex<HashMap<K, Shared<BoxFuture<'static, V>>>>,
}

impl<K: Eq + Hash + Clone + Send + 'static, V: Clone + Send + 'static> Cache<K, V> {
    pub fn new<T, F>(loader: T) -> Self
    where
        F: Future<Output = V> + Sized + Send + 'static,
        T: Fn(K) -> F + Send + Sync + Clone + 'static,
    {
        let loader = Arc::new(loader);

        Self {
            loader: Box::new(move |key| {
                let loader = loader.clone();
                Box::pin(async move { tokio::spawn(loader(key)).await.unwrap() })
            }),
            map: Mutex::new(HashMap::new()),
        }
    }

    pub async fn get(&self, key: K) -> V {
        let mut map = self.map.lock().await;

        let f = map
            .entry(key.clone())
            .or_insert_with(|| (self.loader)(key).boxed().shared())
            .clone();

        drop(map);

        f.await
    }
}
