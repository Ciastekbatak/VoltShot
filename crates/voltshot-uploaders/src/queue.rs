use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Write};
use std::path::Path;
use std::thread;
use std::time::Duration;
use voltshot_core::{CoreError, UploadRequest, UploaderLookup};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
}

impl RetryPolicy {
    /// Returns the delay after a failed attempt, with attempt 1 using the base delay.
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let factor = 2_u64.saturating_pow(attempt.saturating_sub(1));
        Duration::from_millis(
            self.base_delay_ms
                .saturating_mul(factor)
                .min(self.max_delay_ms),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum QueuedUploadStatus {
    Pending,
    InProgress,
    Succeeded { url: String },
    Failed { attempts: u32, last_error: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedUpload {
    pub id: u64,
    pub uploader_id: String,
    #[serde(with = "UploadRequestSnapshot")]
    pub request: UploadRequest,
    pub attempts: u32,
    pub status: QueuedUploadStatus,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "UploadRequest")]
struct UploadRequestSnapshot {
    file_name: String,
    mime_type: String,
    bytes: Vec<u8>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct UploadQueue {
    last_id: u64,
    items: Vec<QueuedUpload>,
}

impl UploadQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn enqueue(&mut self, uploader_id: impl Into<String>, request: UploadRequest) -> u64 {
        self.last_id = self
            .last_id
            .checked_add(1)
            .expect("upload queue ID overflow");
        self.items.push(QueuedUpload {
            id: self.last_id,
            uploader_id: uploader_id.into(),
            request,
            attempts: 0,
            status: QueuedUploadStatus::Pending,
        });
        self.last_id
    }

    /// Processes the oldest pending upload, blocking for retries before returning its status.
    pub fn process_next(
        &mut self,
        lookup: &dyn UploaderLookup,
        policy: &RetryPolicy,
    ) -> Option<&QueuedUpload> {
        let item = self
            .items
            .iter_mut()
            .find(|item| item.status == QueuedUploadStatus::Pending)?;
        item.status = QueuedUploadStatus::InProgress;
        item.status = Self::try_upload(item, lookup, policy);
        Some(item)
    }

    fn try_upload(
        item: &mut QueuedUpload,
        lookup: &dyn UploaderLookup,
        policy: &RetryPolicy,
    ) -> QueuedUploadStatus {
        if item.attempts >= policy.max_attempts {
            return QueuedUploadStatus::Failed {
                attempts: item.attempts,
                last_error: "upload attempt limit reached".into(),
            };
        }
        let Some(uploader) = lookup.get(&item.uploader_id) else {
            return QueuedUploadStatus::Failed {
                attempts: item.attempts,
                last_error: CoreError::UnknownUploader(item.uploader_id.clone()).to_string(),
            };
        };
        loop {
            item.attempts += 1;
            match uploader.upload(&item.request) {
                Ok(result) => return QueuedUploadStatus::Succeeded { url: result.url },
                Err(error)
                    if item.attempts == policy.max_attempts
                        || matches!(error, CoreError::UnknownUploader(_)) =>
                {
                    return QueuedUploadStatus::Failed {
                        attempts: item.attempts,
                        last_error: error.to_string(),
                    };
                }
                Err(_) => thread::sleep(policy.delay_for_attempt(item.attempts)),
            }
        }
    }

    pub fn items(&self) -> &[QueuedUpload] {
        &self.items
    }

    pub fn pending_count(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.status == QueuedUploadStatus::Pending)
            .count()
    }

    pub fn clear_finished(&mut self) {
        self.items.retain(|item| {
            matches!(
                item.status,
                QueuedUploadStatus::Pending | QueuedUploadStatus::InProgress
            )
        });
    }

    /// Saves a JSON snapshot of the queue, including upload payloads.
    pub fn save(&self, path: &Path) -> Result<(), CoreError> {
        let mut nonce = [0; 16];
        getrandom::getrandom(&mut nonce)
            .map_err(|error| CoreError::Io(std::io::Error::other(error.to_string())))?;
        let nonce = u128::from_be_bytes(nonce);
        let temporary_path = path.with_file_name(format!(".voltshot-queue-{nonce:032x}.tmp"));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)?;
        let result = self
            .write_snapshot(file)
            .and_then(|()| fs::rename(&temporary_path, path).map_err(CoreError::from));
        if result.is_err() {
            let _ = fs::remove_file(temporary_path);
        }
        result
    }

    fn write_snapshot(&self, file: File) -> Result<(), CoreError> {
        let mut writer = BufWriter::new(file);
        serde_json::to_writer(&mut writer, self).map_err(|error| {
            CoreError::ConfigError(format!("could not save upload queue: {error}"))
        })?;
        writer.flush()?;
        writer.get_ref().sync_all()?;
        Ok(())
    }

    /// Loads a JSON snapshot and requeues interrupted uploads with their previous attempt counts.
    pub fn load(path: &Path) -> Result<Self, CoreError> {
        let reader = BufReader::new(File::open(path)?);
        let mut queue: Self = serde_json::from_reader(reader).map_err(|error| {
            CoreError::ConfigError(format!("could not load upload queue: {error}"))
        })?;
        let mut previous_id = 0;
        for item in &mut queue.items {
            if item.id <= previous_id || item.id > queue.last_id {
                return Err(CoreError::ConfigError("invalid upload queue IDs".into()));
            }
            previous_id = item.id;
            if item.status == QueuedUploadStatus::InProgress {
                item.status = QueuedUploadStatus::Pending;
            }
        }
        Ok(queue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::UploaderRegistry;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use voltshot_core::{UploadResult, Uploader};

    const TEST_UPLOADER: &str = "test-uploader";
    const RESULT_URL: &str = "https://uploads.example.test/demo.txt";

    struct TestUploader {
        calls: Arc<AtomicU32>,
        transient_failures: u32,
        unknown_uploader: bool,
    }

    impl Uploader for TestUploader {
        fn id(&self) -> &'static str {
            TEST_UPLOADER
        }

        fn name(&self) -> &'static str {
            "Test Uploader"
        }

        fn description(&self) -> &'static str {
            "Returns configured test responses."
        }

        fn upload(&self, request: &UploadRequest) -> Result<UploadResult, CoreError> {
            assert_eq!(request, &request_fixture());
            let attempt = self.calls.fetch_add(1, Ordering::Relaxed) + 1;
            if self.unknown_uploader {
                return Err(CoreError::UnknownUploader("upstream".into()));
            }
            if attempt <= self.transient_failures {
                return Err(CoreError::UploadFailed(format!(
                    "transient error {attempt}"
                )));
            }
            Ok(UploadResult {
                url: RESULT_URL.into(),
            })
        }
    }

    fn registry(
        transient_failures: u32,
        unknown_uploader: bool,
    ) -> (UploaderRegistry, Arc<AtomicU32>) {
        let calls = Arc::new(AtomicU32::new(0));
        let mut registry = UploaderRegistry::new();
        registry.register(Box::new(TestUploader {
            calls: Arc::clone(&calls),
            transient_failures,
            unknown_uploader,
        }));
        (registry, calls)
    }

    fn request_fixture() -> UploadRequest {
        UploadRequest {
            file_name: "demo.txt".into(),
            mime_type: "text/plain".into(),
            bytes: b"queued content".to_vec(),
        }
    }

    fn no_delay_policy(max_attempts: u32) -> RetryPolicy {
        RetryPolicy {
            max_attempts,
            base_delay_ms: 0,
            max_delay_ms: 0,
        }
    }

    #[test]
    fn backoff_doubles_until_capped_and_handles_extreme_inputs() {
        let policy = RetryPolicy {
            max_attempts: 10,
            base_delay_ms: 125,
            max_delay_ms: 750,
        };
        for (attempt, millis) in [
            (0, 125),
            (1, 125),
            (2, 250),
            (3, 500),
            (4, 750),
            (u32::MAX, 750),
        ] {
            assert_eq!(
                policy.delay_for_attempt(attempt),
                Duration::from_millis(millis)
            );
        }
        assert_eq!(
            RetryPolicy {
                max_delay_ms: 100,
                ..policy
            }
            .delay_for_attempt(1),
            Duration::from_millis(100)
        );
        assert_eq!(
            no_delay_policy(u32::MAX).delay_for_attempt(u32::MAX),
            Duration::ZERO
        );
    }

    #[test]
    fn retries_twice_and_stops_at_the_first_success() {
        let (registry, calls) = registry(2, false);
        let mut queue = UploadQueue::new();
        let id = queue.enqueue(TEST_UPLOADER, request_fixture());
        let item = queue.process_next(&registry, &no_delay_policy(5)).unwrap();
        assert_eq!(item.id, id);
        assert_eq!(item.attempts, 3);
        assert_eq!(
            item.status,
            QueuedUploadStatus::Succeeded {
                url: RESULT_URL.into()
            }
        );
        assert_eq!(calls.load(Ordering::Relaxed), 3);
        assert_eq!(queue.pending_count(), 0);
        assert!(queue.process_next(&registry, &no_delay_policy(5)).is_none());
    }

    #[test]
    fn stops_at_the_attempt_limit_and_records_the_final_error() {
        let (registry, calls) = registry(3, false);
        let mut queue = UploadQueue::new();
        queue.enqueue(TEST_UPLOADER, request_fixture());
        let item = queue.process_next(&registry, &no_delay_policy(2)).unwrap();
        assert_eq!(calls.load(Ordering::Relaxed), 2);
        assert_eq!(item.attempts, 2);
        assert_eq!(
            item.status,
            QueuedUploadStatus::Failed {
                attempts: 2,
                last_error: "upload failed: transient error 2".into(),
            }
        );
        assert!(queue.process_next(&registry, &no_delay_policy(2)).is_none());
    }

    #[test]
    fn never_retries_unknown_uploaders() {
        let (registry, calls) = registry(0, true);
        let mut queue = UploadQueue::new();
        queue.enqueue("missing", request_fixture());
        queue.enqueue(TEST_UPLOADER, request_fixture());
        for (missing, attempts) in [("missing", 0), ("upstream", 1)] {
            let item = queue.process_next(&registry, &no_delay_policy(5)).unwrap();
            assert_eq!(item.attempts, attempts);
            assert_eq!(
                item.status,
                QueuedUploadStatus::Failed {
                    attempts,
                    last_error: CoreError::UnknownUploader(missing.into()).to_string(),
                }
            );
        }
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(queue.pending_count(), 0);
    }

    #[test]
    fn honors_a_zero_attempt_budget() {
        let (registry, calls) = registry(0, false);
        let mut queue = UploadQueue::new();
        queue.enqueue(TEST_UPLOADER, request_fixture());
        let item = queue.process_next(&registry, &no_delay_policy(0)).unwrap();
        assert_eq!(item.attempts, 0);
        assert!(matches!(
            item.status,
            QueuedUploadStatus::Failed { attempts: 0, .. }
        ));
        assert_eq!(calls.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn processes_in_fifo_order_and_clears_only_finished_items() {
        let (registry, _) = registry(0, false);
        let mut queue = UploadQueue::new();
        let first = queue.enqueue(TEST_UPLOADER, request_fixture());
        let second = queue.enqueue("missing", request_fixture());
        let third = queue.enqueue(TEST_UPLOADER, request_fixture());
        assert_eq!(queue.pending_count(), 3);

        for id in [first, second] {
            assert_eq!(
                queue
                    .process_next(&registry, &no_delay_policy(1))
                    .unwrap()
                    .id,
                id
            );
        }
        assert_eq!(queue.pending_count(), 1);
        queue.clear_finished();
        assert_eq!(queue.items().len(), 1);
        assert_eq!(queue.items()[0].id, third);
        assert!(queue.enqueue(TEST_UPLOADER, request_fixture()) > third);
    }

    #[test]
    fn snapshots_preserve_payloads_and_resume_interrupted_items() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("upload-queue.json");
        let (registry, calls) = registry(0, false);
        let mut queue = UploadQueue::new();
        let completed = queue.enqueue(TEST_UPLOADER, request_fixture());
        queue.process_next(&registry, &no_delay_policy(1)).unwrap();
        queue.save(&path).unwrap();

        let interrupted = queue.enqueue(TEST_UPLOADER, request_fixture());
        queue.items[1].status = QueuedUploadStatus::InProgress;
        queue.items[1].attempts = 1;
        queue.save(&path).unwrap();

        let mut restored = UploadQueue::load(&path).unwrap();
        assert_eq!(restored.pending_count(), 1);
        assert_eq!(restored.items()[0], queue.items()[0]);
        assert_eq!(restored.items()[0].id, completed);
        assert_eq!(restored.items()[1].request, request_fixture());
        let item = restored
            .process_next(&registry, &no_delay_policy(2))
            .unwrap();
        assert_eq!(item.id, interrupted);
        assert_eq!(item.attempts, 2);
        assert_eq!(
            item.status,
            QueuedUploadStatus::Succeeded {
                url: RESULT_URL.into()
            }
        );
        assert_eq!(calls.load(Ordering::Relaxed), 2);

        restored.clear_finished();
        restored.save(&path).unwrap();
        let mut restored_empty = UploadQueue::load(&path).unwrap();
        assert!(restored_empty.items().is_empty());
        assert!(restored_empty.enqueue(TEST_UPLOADER, request_fixture()) > interrupted);
    }
}
