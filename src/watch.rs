use futures::{
    channel::mpsc::{channel, Receiver},
    SinkExt, StreamExt,
};
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;

fn async_watcher() -> notify::Result<(RecommendedWatcher, Receiver<Event>)> {
    let (mut tx, rx) = channel(1);

    let watcher = RecommendedWatcher::new(
        move |res: notify::Result<Event>| {
            futures::executor::block_on(async {
                if let Ok(res) = res {
                    if res.kind.is_access() {
                        let _ = tx.send(res).await;
                    }
                }
            })
        },
        Config::default(),
    )?;

    Ok((watcher, rx))
}

pub async fn async_watch(paths: impl IntoIterator<Item = &Path>) -> notify::Result<Event> {
    let (mut watcher, mut rx) = async_watcher()?;

    for path in paths {
        watcher.watch(path, RecursiveMode::Recursive)?;
    }

    Ok(rx.next().await.unwrap())
}
