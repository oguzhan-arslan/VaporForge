use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use crate::config::AppConfig;

type BoxFuture = Pin<Box<dyn Future<Output = eyre::Result<()>> + Send + 'static>>;

/// An async scheduled task. Receives a cloned config; returns a pinned future.
pub type Task = Arc<dyn Fn(AppConfig) -> BoxFuture + Send + Sync>;

/// Convenience constructor — boxes and pins the returned future automatically.
pub fn task<F, Fut>(f: F) -> Task
where
    F: Fn(AppConfig) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = eyre::Result<()>> + Send + 'static,
{
    Arc::new(move |cfg| Box::pin(f(cfg)))
}

/// Runs every registered task once. Errors are logged; they never stop
/// subsequent tasks from running.
pub async fn run_tasks(config: &AppConfig, tasks: &[Task]) {
    for t in tasks {
        if let Err(e) = t(config.clone()).await {
            tracing::error!("scheduled task error: {e:#}");
        }
    }
}

/// Runs the scheduler loop indefinitely.
///
/// Sleeps for `config.scheduler.scan_interval_minutes`, then calls every task.
/// New tasks can be registered by pushing to `tasks` before calling this —
/// the loop itself never needs to change.
///
/// Returns only if the sleep is cancelled (process shutdown).
pub async fn run(config: AppConfig, tasks: Vec<Task>) {
    let interval = Duration::from_secs(config.scheduler.scan_interval_minutes * 60);
    tracing::info!(
        "scheduler started — interval {} min",
        config.scheduler.scan_interval_minutes
    );
    loop {
        tokio::time::sleep(interval).await;
        run_tasks(&config, &tasks).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn run_tasks_calls_every_task() {
        let counter = Arc::new(AtomicUsize::new(0));
        let c1 = counter.clone();
        let c2 = counter.clone();

        let tasks: Vec<Task> = vec![
            task(move |_| { let c = c1.clone(); async move { c.fetch_add(1, Ordering::SeqCst); Ok(()) } }),
            task(move |_| { let c = c2.clone(); async move { c.fetch_add(1, Ordering::SeqCst); Ok(()) } }),
        ];

        run_tasks(&AppConfig::default(), &tasks).await;
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn run_tasks_continues_after_error() {
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();

        let tasks: Vec<Task> = vec![
            task(|_| async move { eyre::bail!("task 1 failed") }),
            task(move |_| { let c = c.clone(); async move { c.fetch_add(1, Ordering::SeqCst); Ok(()) } }),
        ];

        run_tasks(&AppConfig::default(), &tasks).await;
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn run_executes_task_after_one_tick() {
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();

        let mut config = AppConfig::default();
        config.scheduler.scan_interval_minutes = 1;

        let tasks = vec![task(move |_| {
            let c = c.clone();
            async move { c.fetch_add(1, Ordering::SeqCst); Ok(()) }
        })];

        let handle = tokio::spawn(run(config, tasks));

        tokio::time::sleep(Duration::from_secs(61)).await;

        handle.abort();
        assert!(counter.load(Ordering::SeqCst) >= 1);
    }
}
