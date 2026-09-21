use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber;

use super::selected_text_runtime::read_observable_clipboard_text;

const POLL_INTERVAL: Duration = Duration::from_millis(40);

static INTERNAL_CLIPBOARD_OPERATION_DEPTH: AtomicU32 = AtomicU32::new(0);
static LAST_INTERNAL_CLIPBOARD_SEQUENCE: AtomicU32 = AtomicU32::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardListenerError {
    ThreadSpawnFailed,
}

pub struct ClipboardTextListener {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl ClipboardTextListener {
    pub fn start(
        mut observer: impl FnMut(String) + Send + 'static,
    ) -> Result<Self, ClipboardListenerError> {
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let initial_sequence = unsafe { GetClipboardSequenceNumber() };
        let worker = thread::Builder::new()
            .name("sunswitcher-clipboard-listener".to_owned())
            .spawn(move || {
                let mut last_seen = initial_sequence;
                while !worker_stop.load(Ordering::Acquire) {
                    thread::sleep(POLL_INTERVAL);
                    let current = unsafe { GetClipboardSequenceNumber() };
                    if !should_observe_clipboard_sequence(
                        last_seen,
                        current,
                        INTERNAL_CLIPBOARD_OPERATION_DEPTH.load(Ordering::Acquire),
                        LAST_INTERNAL_CLIPBOARD_SEQUENCE.load(Ordering::Acquire),
                    ) {
                        if current != last_seen {
                            last_seen = current;
                        }
                        continue;
                    }

                    let clipboard_text = match read_observable_clipboard_text() {
                        Ok(text) => text,
                        Err(_) => {
                            // Another process may still own the clipboard immediately after the
                            // sequence changes. Keep this sequence pending and retry on the next tick.
                            continue;
                        }
                    };
                    let after_read = unsafe { GetClipboardSequenceNumber() };
                    let internal_depth = INTERNAL_CLIPBOARD_OPERATION_DEPTH.load(Ordering::Acquire);
                    let last_internal_sequence =
                        LAST_INTERNAL_CLIPBOARD_SEQUENCE.load(Ordering::Acquire);
                    if !should_deliver_clipboard_read(
                        current,
                        after_read,
                        internal_depth,
                        last_internal_sequence,
                    ) {
                        if after_read == current
                            && (internal_depth != 0 || after_read == last_internal_sequence)
                        {
                            last_seen = after_read;
                        }
                        continue;
                    }

                    last_seen = current;
                    if let Some(text) = clipboard_text
                        && !text.is_empty()
                    {
                        observer(text);
                    }
                }
            })
            .map_err(|_| ClipboardListenerError::ThreadSpawnFailed)?;
        Ok(Self {
            stop,
            worker: Some(worker),
        })
    }
}

impl Drop for ClipboardTextListener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub(super) struct InternalClipboardMutationGuard;

impl InternalClipboardMutationGuard {
    pub(super) fn begin() -> Self {
        INTERNAL_CLIPBOARD_OPERATION_DEPTH.fetch_add(1, Ordering::AcqRel);
        Self
    }
}

impl Drop for InternalClipboardMutationGuard {
    fn drop(&mut self) {
        LAST_INTERNAL_CLIPBOARD_SEQUENCE
            .store(unsafe { GetClipboardSequenceNumber() }, Ordering::Release);
        INTERNAL_CLIPBOARD_OPERATION_DEPTH.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(super) fn should_observe_clipboard_sequence(
    last_seen: u32,
    current: u32,
    internal_depth: u32,
    last_internal_sequence: u32,
) -> bool {
    current != last_seen && internal_depth == 0 && current != last_internal_sequence
}

pub(super) fn should_deliver_clipboard_read(
    expected_sequence: u32,
    after_read_sequence: u32,
    internal_depth: u32,
    last_internal_sequence: u32,
) -> bool {
    expected_sequence == after_read_sequence
        && internal_depth == 0
        && after_read_sequence != last_internal_sequence
}
