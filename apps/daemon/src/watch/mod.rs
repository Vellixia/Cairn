//! T042: watcher manager. One notify watcher per live-session worktree;
//! notifications are ADVISORY hints only — reconciliation against Git state
//! produces every authoritative snapshot (arch rules 7–8).

pub mod debounce;
pub mod filter;
pub mod reconcile;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use cairn_domain::WatcherStartStage;
use notify::{RecursiveMode, Watcher};
use tokio::sync::{mpsc, oneshot, watch, Notify};

use crate::state::AppState;

#[derive(Debug)]
pub enum WatchCommand {
    Watch {
        repository_id: String,
        worktree_id: String,
        root: PathBuf,
        ready: oneshot::Sender<Result<(), WatchStartFailure>>,
    },
    Unwatch {
        worktree_id: String,
    },
}

struct ActiveWatch {
    stop: watch::Sender<bool>,
    actions: mpsc::UnboundedSender<WatchAction>,
}

#[derive(Debug, Clone)]
pub struct WatchStartFailure {
    pub stage: WatcherStartStage,
    pub message: String,
}

impl WatchStartFailure {
    fn install(message: impl Into<String>) -> Self {
        Self {
            stage: WatcherStartStage::Install,
            message: message.into(),
        }
    }

    fn reconcile(message: impl Into<String>) -> Self {
        Self {
            stage: WatcherStartStage::Reconcile,
            message: message.into(),
        }
    }
}

enum WatchAction {
    Reconcile {
        done: oneshot::Sender<Result<(), WatchStartFailure>>,
    },
}

/// Deterministic coordination points for integration tests. Production
/// configuration leaves this absent, so the fast path has no pauses or
/// injected failures (T064).
#[derive(Debug, Default)]
pub struct WatcherTestControls {
    pause_before_install: AtomicBool,
    before_install_reached: Notify,
    release_install: Notify,
    pause_before_reconcile: AtomicBool,
    before_reconcile_reached: Notify,
    release_reconcile: Notify,
    installed: Notify,
    installed_count: AtomicU64,
    reconciled: Notify,
    reconciled_count: AtomicU64,
    fail_install: AtomicBool,
    fail_reconcile: AtomicBool,
    drop_notifications: AtomicBool,
}

impl WatcherTestControls {
    pub fn pause_before_install(&self) {
        self.pause_before_install.store(true, Ordering::SeqCst);
    }

    pub async fn wait_before_install(&self) {
        self.before_install_reached.notified().await;
    }

    pub fn release_install(&self) {
        self.release_install.notify_one();
    }

    pub fn pause_before_reconcile(&self) {
        self.pause_before_reconcile.store(true, Ordering::SeqCst);
    }

    pub async fn wait_before_reconcile(&self) {
        self.before_reconcile_reached.notified().await;
    }

    pub fn release_reconcile(&self) {
        self.release_reconcile.notify_one();
    }

    pub fn installed_count(&self) -> u64 {
        self.installed_count.load(Ordering::SeqCst)
    }

    pub async fn wait_installed_after(&self, baseline: u64) {
        loop {
            let notified = self.installed.notified();
            if self.installed_count() > baseline {
                return;
            }
            notified.await;
        }
    }

    pub fn reconciled_count(&self) -> u64 {
        self.reconciled_count.load(Ordering::SeqCst)
    }

    pub async fn wait_reconciled_after(&self, baseline: u64) {
        loop {
            let notified = self.reconciled.notified();
            if self.reconciled_count() > baseline {
                return;
            }
            notified.await;
        }
    }

    pub fn force_install_failure(&self) {
        self.fail_install.store(true, Ordering::SeqCst);
    }

    pub fn force_reconcile_failure(&self) {
        self.fail_reconcile.store(true, Ordering::SeqCst);
    }

    /// Suppress watcher callbacks until `resume_notifications` is called.
    /// This models notification loss deterministically; an explicit Git
    /// reconciliation must still recover the authoritative state.
    pub fn drop_notifications(&self) {
        self.drop_notifications.store(true, Ordering::SeqCst);
    }

    pub fn resume_notifications(&self) {
        self.drop_notifications.store(false, Ordering::SeqCst);
    }

    async fn before_install(&self) {
        if self.pause_before_install.swap(false, Ordering::SeqCst) {
            self.before_install_reached.notify_one();
            self.release_install.notified().await;
        }
    }

    async fn before_reconcile(&self) {
        if self.pause_before_reconcile.swap(false, Ordering::SeqCst) {
            self.before_reconcile_reached.notify_one();
            self.release_reconcile.notified().await;
        }
    }

    fn note_installed(&self) {
        self.installed_count.fetch_add(1, Ordering::SeqCst);
        self.installed.notify_waiters();
    }

    fn note_reconciled(&self) {
        self.reconciled_count.fetch_add(1, Ordering::SeqCst);
        self.reconciled.notify_waiters();
    }
}

pub async fn request_ready(
    state: &AppState,
    repository_id: String,
    worktree_id: String,
    root: PathBuf,
) -> Result<(), WatchStartFailure> {
    let (ready, response) = oneshot::channel();
    state
        .inner
        .watch_tx
        .send(WatchCommand::Watch {
            repository_id,
            worktree_id,
            root,
            ready,
        })
        .map_err(|_| WatchStartFailure::install("watcher manager is unavailable"))?;
    response
        .await
        .map_err(|_| WatchStartFailure::install("watcher task ended before readiness"))?
}

pub async fn manager_loop(state: AppState, mut shutdown: watch::Receiver<bool>) {
    let mut rx = match state.inner.watch_rx.lock().expect("watch rx lock").take() {
        Some(rx) => rx,
        None => return, // second manager instance: nothing to do
    };
    let mut active: HashMap<String, ActiveWatch> = HashMap::new();

    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            cmd = rx.recv() => {
                let Some(cmd) = cmd else { break };
                match cmd {
                    WatchCommand::Watch { repository_id, worktree_id, root, ready } => {
                        if let Some(existing) = active.get(&worktree_id) {
                            let (done, response) = oneshot::channel();
                            if existing.actions.send(WatchAction::Reconcile { done }).is_err() {
                                let _ = ready.send(Err(WatchStartFailure::install(
                                    "existing watcher task is unavailable",
                                )));
                                continue;
                            }
                            tokio::spawn(async move {
                                let result = response.await.unwrap_or_else(|_| {
                                    Err(WatchStartFailure::reconcile(
                                        "existing watcher ended during reconciliation",
                                    ))
                                });
                                let _ = ready.send(result);
                            });
                            continue;
                        }
                        let (stop_tx, stop_rx) = watch::channel(false);
                        let (action_tx, action_rx) = mpsc::unbounded_channel();
                        let (started_tx, started_rx) = oneshot::channel();
                        tokio::spawn(worktree_task(
                            state.clone(),
                            repository_id,
                            worktree_id.clone(),
                            root,
                            stop_rx,
                            action_rx,
                            started_tx,
                        ));
                        let result = started_rx.await.unwrap_or_else(|_| {
                            Err(WatchStartFailure::install(
                                "watcher task ended before readiness",
                            ))
                        });
                        if result.is_ok() {
                            active.insert(
                                worktree_id,
                                ActiveWatch {
                                    stop: stop_tx,
                                    actions: action_tx,
                                },
                            );
                            state.inner.watched.store(active.len() as u64, Ordering::Relaxed);
                        }
                        let _ = ready.send(result);
                    }
                    WatchCommand::Unwatch { worktree_id } => {
                        if let Some(w) = active.remove(&worktree_id) {
                            let _ = w.stop.send(true);
                        }
                        state.inner.watched.store(active.len() as u64, Ordering::Relaxed);
                    }
                }
            }
        }
    }
    for (_, w) in active.drain() {
        let _ = w.stop.send(true);
    }
}

/// One worktree's watch → debounce → reconcile pipeline (T042–T044).
async fn worktree_task(
    state: AppState,
    repository_id: String,
    worktree_id: String,
    root: PathBuf,
    mut stop: watch::Receiver<bool>,
    mut actions: mpsc::UnboundedReceiver<WatchAction>,
    started: oneshot::Sender<Result<(), WatchStartFailure>>,
) {
    let (evt_tx, evt_rx) = mpsc::unbounded_channel::<notify::Result<notify::Event>>();
    let controls = state.inner.config.watcher_test_controls.clone();
    if let Some(c) = &controls {
        c.before_install().await;
        if c.fail_install.swap(false, Ordering::SeqCst) {
            let _ = started.send(Err(WatchStartFailure::install(
                "watcher installation failed by deterministic test injection",
            )));
            return;
        }
    }
    let callback_controls = controls.clone();
    let mut watcher = match notify::recommended_watcher(move |res| {
        if callback_controls
            .as_ref()
            .is_some_and(|c| c.drop_notifications.load(Ordering::SeqCst))
        {
            return;
        }
        let _ = evt_tx.send(res);
    }) {
        Ok(w) => w,
        Err(e) => {
            tracing::warn!(error = %e, "failed to create watcher");
            let _ = started.send(Err(WatchStartFailure::install(e.to_string())));
            return;
        }
    };
    if let Err(e) = watcher.watch(&root, RecursiveMode::Recursive) {
        tracing::warn!(error = %e, root = %root.display(), "failed to watch worktree");
        let _ = started.send(Err(WatchStartFailure::install(e.to_string())));
        return;
    }
    // Linked worktrees keep HEAD/index in a git dir outside the root: watch
    // it too so branch switches with no worktree file changes are seen.
    if let Ok(layout) = cairn_git::discover::discover(&root).await {
        if !layout.git_dir.starts_with(&root) {
            let _ = watcher.watch(&layout.git_dir, RecursiveMode::Recursive);
        }
    }

    if let Some(c) = &controls {
        c.note_installed();
        c.before_reconcile().await;
    }

    let quiescence = std::time::Duration::from_millis(state.inner.config.debounce_quiescence_ms);
    let deadline = std::time::Duration::from_millis(state.inner.config.debounce_deadline_ms);
    let mut debouncer = debounce::Debouncer::new(quiescence, deadline);
    let mut reconciler = reconcile::Reconciler::new(
        state.clone(),
        repository_id.clone(),
        worktree_id.clone(),
        root.clone(),
    )
    .await;
    let mut evt_rx = evt_rx;

    let initial = if controls
        .as_ref()
        .is_some_and(|c| c.fail_reconcile.swap(false, Ordering::SeqCst))
    {
        Err(WatchStartFailure::reconcile(
            "watcher reconciliation failed by deterministic test injection",
        ))
    } else {
        reconciler
            .reconcile()
            .await
            .map(|_| ())
            .map_err(|e| WatchStartFailure::reconcile(e.message))
    };
    if let Some(c) = &controls {
        c.note_reconciled();
    }
    if initial.is_err() {
        let _ = started.send(initial);
        return;
    }
    let _ = started.send(Ok(()));

    tracing::info!(worktree = %worktree_id, root = %root.display(), "watching");
    loop {
        let wait = debouncer.next_deadline();
        tokio::select! {
            _ = stop.changed() => break,
            action = actions.recv() => {
                let Some(WatchAction::Reconcile { done }) = action else { break };
                if let Some(c) = &controls {
                    c.before_reconcile().await;
                }
                let result = reconciler
                    .reconcile()
                    .await
                    .map(|_| ())
                    .map_err(|e| WatchStartFailure::reconcile(e.message));
                if let Some(c) = &controls {
                    c.note_reconciled();
                }
                let _ = done.send(result);
            }
            evt = evt_rx.recv() => {
                match evt {
                    Some(Ok(event)) => {
                        if filter::relevant(&root, &event) {
                            debouncer.record();
                        }
                    }
                    Some(Err(_)) | None => {
                        // Watcher overflow/error: hints are droppable — force
                        // a full reconcile (research R9).
                        debouncer.record();
                    }
                }
            }
            _ = debounce::sleep_until_opt(wait) => {
                if debouncer.should_fire() {
                    debouncer.reset();
                    if let Err(e) = reconciler.reconcile().await {
                        tracing::warn!(worktree = %worktree_id, error = %e.message, "reconcile failed");
                    } else if let Some(c) = &controls {
                        c.note_reconciled();
                    }
                }
            }
        }
    }
    tracing::info!(worktree = %worktree_id, "watch stopped");
}
