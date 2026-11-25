use eframe::egui;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use voltshot_core::{WatchFolderRule, WatchFolderScanner, WatchedFileEvent};

/// Default polling interval used by `WatchFolderRuntime` when scanning watched directories.
pub const DEFAULT_WATCH_POLL_INTERVAL: Duration = Duration::from_millis(1500);

const MIN_POLL_INTERVAL: Duration = Duration::from_millis(10);

enum ControlMessage {
    SyncRules(Vec<WatchFolderRule>),
    Shutdown,
}

/// Background watch-folder daemon driving `WatchFolderScanner` off the UI thread.
pub struct WatchFolderRuntime {
    control_tx: Option<Sender<ControlMessage>>,
    event_rx: Receiver<WatchedFileEvent>,
    running: Arc<AtomicBool>,
    stop_flag: Arc<AtomicBool>,
    watched_count: Arc<AtomicUsize>,
    last_error: Arc<Mutex<Option<String>>>,
    worker: Option<JoinHandle<()>>,
}

impl WatchFolderRuntime {
    /// Starts the background watch-folder daemon with the default 1500 ms poll interval.
    pub fn new(ctx: egui::Context, rules: Vec<WatchFolderRule>) -> Self {
        Self::with_poll_interval(Some(ctx), rules, DEFAULT_WATCH_POLL_INTERVAL)
    }

    /// Starts the background watch-folder daemon with a custom poll interval and optional `egui::Context`.
    pub fn with_poll_interval(
        ctx: Option<egui::Context>,
        rules: Vec<WatchFolderRule>,
        poll_interval: Duration,
    ) -> Self {
        let interval = poll_interval.max(MIN_POLL_INTERVAL);
        let enabled_count = count_enabled_rules(&rules);
        let (control_tx, control_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();

        let running = Arc::new(AtomicBool::new(true));
        let stop_flag = Arc::new(AtomicBool::new(false));
        let watched_count = Arc::new(AtomicUsize::new(enabled_count));
        let last_error = Arc::new(Mutex::new(None));

        let running_bg = Arc::clone(&running);
        let stop_flag_bg = Arc::clone(&stop_flag);
        let watched_count_bg = Arc::clone(&watched_count);
        let last_error_bg = Arc::clone(&last_error);

        let worker_args = WatchLoopArgs {
            ctx,
            rules,
            poll_interval: interval,
            control_rx,
            event_tx,
            running: running_bg,
            stop_flag: stop_flag_bg,
            watched_count: watched_count_bg,
            last_error: last_error_bg,
        };

        let worker = match thread::Builder::new()
            .name("voltshot-watch-folder".into())
            .spawn(move || {
                run_watch_loop(worker_args);
            }) {
            Ok(handle) => Some(handle),
            Err(err) => {
                running.store(false, Ordering::Release);
                if let Ok(mut guard) = last_error.lock() {
                    *guard = Some(format!("Failed to start watch folder daemon: {err}"));
                }
                None
            }
        };

        Self {
            control_tx: Some(control_tx),
            event_rx,
            running,
            stop_flag,
            watched_count,
            last_error,
            worker,
        }
    }

    /// Replaces the active watch-folder rules and immediately triggers a scan if any rules are enabled.
    pub fn sync_rules(&mut self, rules: Vec<WatchFolderRule>) {
        let enabled_count = count_enabled_rules(&rules);
        self.watched_count.store(enabled_count, Ordering::Release);
        if let Some(tx) = &self.control_tx {
            let _ = tx.send(ControlMessage::SyncRules(rules));
        }
    }

    /// Drains all newly detected `WatchedFileEvent`s emitted by the background scanner.
    pub fn poll_events(&self) -> Vec<WatchedFileEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.event_rx.try_recv() {
            events.push(event);
        }
        events
    }

    /// Returns `true` while the background daemon is running with at least one enabled watch rule.
    pub fn is_active(&self) -> bool {
        self.running.load(Ordering::Acquire) && self.watched_count() > 0
    }

    /// Returns the number of currently enabled watch-folder rules.
    pub fn watched_count(&self) -> usize {
        self.watched_count.load(Ordering::Acquire)
    }

    /// Returns the most recent scan error recorded by the daemon, if any.
    pub fn last_error(&self) -> Option<String> {
        self.last_error.lock().ok().and_then(|guard| guard.clone())
    }
}

impl Drop for WatchFolderRuntime {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::Release);
        if let Some(tx) = self.control_tx.take() {
            let _ = tx.send(ControlMessage::Shutdown);
        }
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
        self.running.store(false, Ordering::Release);
    }
}

fn count_enabled_rules(rules: &[WatchFolderRule]) -> usize {
    rules.iter().filter(|rule| rule.enabled).count()
}

struct WatchLoopArgs {
    ctx: Option<egui::Context>,
    rules: Vec<WatchFolderRule>,
    poll_interval: Duration,
    control_rx: Receiver<ControlMessage>,
    event_tx: Sender<WatchedFileEvent>,
    running: Arc<AtomicBool>,
    stop_flag: Arc<AtomicBool>,
    watched_count: Arc<AtomicUsize>,
    last_error: Arc<Mutex<Option<String>>>,
}

fn run_watch_loop(args: WatchLoopArgs) {
    let WatchLoopArgs {
        ctx,
        mut rules,
        poll_interval,
        control_rx,
        event_tx,
        running,
        stop_flag,
        watched_count,
        last_error,
    } = args;
    let mut scanner = WatchFolderScanner::new();
    scan_and_dispatch(&mut scanner, &rules, &event_tx, ctx.as_ref(), &last_error);

    while !stop_flag.load(Ordering::Acquire) {
        let has_enabled = rules.iter().any(|rule| rule.enabled);
        let recv_result = if has_enabled {
            match control_rx.recv_timeout(poll_interval) {
                Ok(msg) => Some(msg),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match control_rx.recv() {
                Ok(msg) => Some(msg),
                Err(_) => break,
            }
        };

        if let Some(msg) = recv_result {
            let mut should_stop = false;
            let mut rules_updated = false;
            let mut pending = Some(msg);

            while let Some(current) = pending.take() {
                match current {
                    ControlMessage::SyncRules(next_rules) => {
                        rules = next_rules;
                        rules_updated = true;
                    }
                    ControlMessage::Shutdown => {
                        should_stop = true;
                        break;
                    }
                }
                pending = control_rx.try_recv().ok();
            }

            if should_stop || stop_flag.load(Ordering::Acquire) {
                break;
            }

            if rules_updated {
                let enabled = count_enabled_rules(&rules);
                watched_count.store(enabled, Ordering::Release);
                if enabled > 0 {
                    scan_and_dispatch(&mut scanner, &rules, &event_tx, ctx.as_ref(), &last_error);
                }
            }
        } else if !stop_flag.load(Ordering::Acquire) {
            scan_and_dispatch(&mut scanner, &rules, &event_tx, ctx.as_ref(), &last_error);
        }
    }

    running.store(false, Ordering::Release);
}

fn scan_and_dispatch(
    scanner: &mut WatchFolderScanner,
    rules: &[WatchFolderRule],
    event_tx: &Sender<WatchedFileEvent>,
    ctx: Option<&egui::Context>,
    last_error: &Arc<Mutex<Option<String>>>,
) {
    let mut emitted_any = false;
    let mut first_error: Option<String> = None;

    for rule in rules {
        if !rule.enabled {
            continue;
        }
        match scanner.scan(std::slice::from_ref(rule)) {
            Ok(events) => {
                for event in events {
                    if event_tx.send(event).is_ok() {
                        emitted_any = true;
                    }
                }
            }
            Err(err) => {
                if first_error.is_none() {
                    first_error = Some(format!("Watch rule `{}`: {err}", rule.id));
                }
            }
        }
    }

    if let Ok(mut guard) = last_error.lock() {
        *guard = first_error;
    }

    if emitted_any && let Some(egui_ctx) = ctx {
        egui_ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Instant;
    use tempfile::tempdir;

    #[test]
    fn watch_folder_runtime_detects_new_png_wakes_only_on_event_and_shuts_down_cleanly() {
        let watched_dir = tempdir().expect("tempdir");
        let repaint_count = Arc::new(AtomicUsize::new(0));
        let repaint_count_cb = Arc::clone(&repaint_count);

        let ctx = egui::Context::default();
        ctx.set_request_repaint_callback(move |_| {
            repaint_count_cb.fetch_add(1, Ordering::AcqRel);
        });

        let rule = WatchFolderRule {
            id: "incoming-pngs".into(),
            folder: watched_dir.path().to_path_buf(),
            extensions: vec!["png".into()],
            move_after_process: None,
            enabled: true,
            workflow_id: Some("quick-upload".into()),
        };

        let mut runtime = WatchFolderRuntime::with_poll_interval(
            Some(ctx),
            vec![rule],
            Duration::from_millis(20),
        );
        assert!(runtime.is_active());
        assert_eq!(runtime.watched_count(), 1);

        thread::sleep(Duration::from_millis(90));
        assert!(runtime.poll_events().is_empty());
        assert_eq!(repaint_count.load(Ordering::Acquire), 0);

        let png_path = watched_dir.path().join("fresh_capture.png");
        let txt_path = watched_dir.path().join("ignored.txt");
        fs::write(&txt_path, b"not-an-image").expect("write txt");
        fs::write(&png_path, b"fake-png-bytes").expect("write png");

        let deadline = Instant::now() + Duration::from_secs(2);
        let mut received = Vec::new();
        while Instant::now() < deadline {
            received.extend(runtime.poll_events());
            if !received.is_empty() {
                break;
            }
            thread::sleep(Duration::from_millis(15));
        }

        assert_eq!(
            received,
            vec![WatchedFileEvent {
                rule_id: "incoming-pngs".into(),
                path: png_path,
                workflow_id: Some("quick-upload".into()),
            }]
        );
        let repaints_after_event = repaint_count.load(Ordering::Acquire);
        assert_eq!(repaints_after_event, 1);

        thread::sleep(Duration::from_millis(80));
        assert!(runtime.poll_events().is_empty());
        assert_eq!(repaint_count.load(Ordering::Acquire), repaints_after_event);

        runtime.sync_rules(Vec::new());
        assert!(!runtime.is_active());
        assert_eq!(runtime.watched_count(), 0);

        let drop_start = Instant::now();
        drop(runtime);
        assert!(drop_start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn watch_folder_runtime_interrupts_long_poll_interval_immediately_on_drop() {
        let watched_dir = tempdir().expect("tempdir");
        let rule = WatchFolderRule {
            id: "slow-poll".into(),
            folder: watched_dir.path().to_path_buf(),
            extensions: vec!["png".into()],
            move_after_process: None,
            enabled: true,
            workflow_id: None,
        };

        let runtime =
            WatchFolderRuntime::with_poll_interval(None, vec![rule], Duration::from_secs(30));
        assert!(runtime.is_active());
        assert_eq!(runtime.watched_count(), 1);

        let drop_start = Instant::now();
        drop(runtime);
        assert!(drop_start.elapsed() < Duration::from_secs(2));
    }
}
