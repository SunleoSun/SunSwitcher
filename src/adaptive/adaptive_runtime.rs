use std::error::Error;
use std::fmt::{Display, Formatter};
use std::sync::{Arc, Mutex, RwLock, mpsc};
use std::thread::{self, JoinHandle};

use crate::correction::{
    Confidence, CorrectionDecision, CorrectionEngine, LexicalCorrectionProvider, LexicalSnapshot,
};
use crate::input::{Boundary, CompletedToken, InputBuffer, InputEvent, InputOutcome};
use crate::language::normalize_word;
use crate::lexicon::UserLexicon;
use crate::persistence::{CorrectionEventId, CorrectionUndoPlan, Database, DatabaseError};
use crate::replacement::{ReplacementAction, ReplacementEngine, ReplacementOutcome, UndoOutcome};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdaptiveCorrectionDirective {
    Pass,
    Replace(ReplacementAction),
}

#[derive(Debug)]
pub enum AdaptiveRuntimeError {
    Database(DatabaseError),
    NoLanguages,
    SnapshotUnavailable,
    LearningWorkerUnavailable,
    LearningWorkerFailed(String),
}

impl Display for AdaptiveRuntimeError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Database(error) => write!(formatter, "database error: {error}"),
            Self::NoLanguages => formatter.write_str("adaptive lexical snapshot has no languages"),
            Self::SnapshotUnavailable => {
                formatter.write_str("adaptive lexical snapshot is unavailable")
            }
            Self::LearningWorkerUnavailable => {
                formatter.write_str("adaptive learning worker is unavailable")
            }
            Self::LearningWorkerFailed(error) => {
                write!(formatter, "adaptive learning worker failed: {error}")
            }
        }
    }
}

impl Error for AdaptiveRuntimeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Database(error) => Some(error),
            _ => None,
        }
    }
}

impl From<DatabaseError> for AdaptiveRuntimeError {
    fn from(error: DatabaseError) -> Self {
        Self::Database(error)
    }
}

#[derive(Debug, Clone)]
pub struct LexicalSnapshotStore {
    current: Arc<RwLock<Arc<LexicalSnapshot>>>,
}

impl LexicalSnapshotStore {
    fn new(snapshot: LexicalSnapshot) -> Self {
        Self {
            current: Arc::new(RwLock::new(Arc::new(snapshot))),
        }
    }

    pub fn load(&self) -> Result<Arc<LexicalSnapshot>, AdaptiveRuntimeError> {
        self.current
            .read()
            .map(|snapshot| Arc::clone(&snapshot))
            .map_err(|_| AdaptiveRuntimeError::SnapshotUnavailable)
    }

    fn replace_user_lexicon(&self, user_lexicon: UserLexicon) -> Result<(), AdaptiveRuntimeError> {
        let current = self.load()?;
        let replacement = Arc::new(current.with_user_lexicon(user_lexicon));
        let mut slot = self
            .current
            .write()
            .map_err(|_| AdaptiveRuntimeError::SnapshotUnavailable)?;
        *slot = replacement;
        Ok(())
    }
}

#[derive(Debug)]
enum LearningCommand {
    ObserveToken {
        token: String,
        used_at_ms: i64,
    },
    ObserveText {
        text: String,
        used_at_ms: i64,
    },
    RecordCorrection {
        observed_text: String,
        replacement_text: String,
        created_at_ms: i64,
        response: mpsc::Sender<Result<CorrectionEventId, String>>,
    },
    PrepareUndo {
        event_id: CorrectionEventId,
        response: mpsc::Sender<Result<CorrectionUndoPlan, String>>,
    },
    CommitUndo {
        event_id: CorrectionEventId,
        undone_at_ms: i64,
        response: mpsc::Sender<Result<(), String>>,
    },
    CommitRecordedUndo {
        receipt: CorrectionEventReceipt,
        undone_at_ms: i64,
    },
    Barrier(mpsc::Sender<()>),
    Shutdown,
}

#[derive(Debug)]
struct CorrectionEventReceipt {
    response: mpsc::Receiver<Result<CorrectionEventId, String>>,
}

#[derive(Debug, Clone)]
pub struct LearningClient {
    sender: mpsc::Sender<LearningCommand>,
    last_error: Arc<Mutex<Option<String>>>,
}

impl LearningClient {
    pub fn observe_typed_token(
        &self,
        token: impl Into<String>,
        used_at_ms: i64,
    ) -> Result<(), AdaptiveRuntimeError> {
        self.send(LearningCommand::ObserveToken {
            token: token.into(),
            used_at_ms,
        })
    }

    pub fn observe_text(
        &self,
        text: impl Into<String>,
        used_at_ms: i64,
    ) -> Result<(), AdaptiveRuntimeError> {
        self.send(LearningCommand::ObserveText {
            text: text.into(),
            used_at_ms,
        })
    }

    fn record_correction(
        &self,
        observed_text: impl Into<String>,
        replacement_text: impl Into<String>,
        created_at_ms: i64,
    ) -> Result<CorrectionEventReceipt, AdaptiveRuntimeError> {
        let (response, receiver) = mpsc::channel();
        self.send(LearningCommand::RecordCorrection {
            observed_text: observed_text.into(),
            replacement_text: replacement_text.into(),
            created_at_ms,
            response,
        })?;
        Ok(CorrectionEventReceipt { response: receiver })
    }

    pub fn prepare_undo(
        &self,
        event_id: CorrectionEventId,
    ) -> Result<CorrectionUndoPlan, AdaptiveRuntimeError> {
        let (response, receiver) = mpsc::channel();
        self.send(LearningCommand::PrepareUndo { event_id, response })?;
        receiver
            .recv()
            .map_err(|_| AdaptiveRuntimeError::LearningWorkerUnavailable)?
            .map_err(AdaptiveRuntimeError::LearningWorkerFailed)
    }

    pub fn commit_undo(
        &self,
        event_id: CorrectionEventId,
        undone_at_ms: i64,
    ) -> Result<(), AdaptiveRuntimeError> {
        let (response, receiver) = mpsc::channel();
        self.send(LearningCommand::CommitUndo {
            event_id,
            undone_at_ms,
            response,
        })?;
        receiver
            .recv()
            .map_err(|_| AdaptiveRuntimeError::LearningWorkerUnavailable)?
            .map_err(AdaptiveRuntimeError::LearningWorkerFailed)
    }

    fn queue_commit_recorded_undo(
        &self,
        receipt: CorrectionEventReceipt,
        undone_at_ms: i64,
    ) -> Result<(), AdaptiveRuntimeError> {
        self.send(LearningCommand::CommitRecordedUndo {
            receipt,
            undone_at_ms,
        })
    }

    pub fn flush(&self) -> Result<(), AdaptiveRuntimeError> {
        let (barrier, receiver) = mpsc::channel();
        self.send(LearningCommand::Barrier(barrier))?;
        receiver
            .recv()
            .map_err(|_| AdaptiveRuntimeError::LearningWorkerUnavailable)?;
        let last_error = self
            .last_error
            .lock()
            .map_err(|_| AdaptiveRuntimeError::LearningWorkerUnavailable)?;
        match last_error.as_ref() {
            Some(error) => Err(AdaptiveRuntimeError::LearningWorkerFailed(error.clone())),
            None => Ok(()),
        }
    }

    fn send(&self, command: LearningCommand) -> Result<(), AdaptiveRuntimeError> {
        self.sender
            .send(command)
            .map_err(|_| AdaptiveRuntimeError::LearningWorkerUnavailable)
    }
}

pub struct AdaptiveLexicalRuntime {
    snapshots: LexicalSnapshotStore,
    learning: LearningClient,
    minimum_confidence: Confidence,
    worker: Option<JoinHandle<()>>,
}

impl AdaptiveLexicalRuntime {
    pub fn start(
        database: Database,
        minimum_confidence: Confidence,
    ) -> Result<Self, AdaptiveRuntimeError> {
        let languages = database.load_enabled_language_packs()?;
        let user_lexicon = database.load_user_lexicon()?;
        let snapshot = LexicalSnapshot::try_new(languages, user_lexicon)
            .map_err(|_| AdaptiveRuntimeError::NoLanguages)?;
        let snapshots = LexicalSnapshotStore::new(snapshot);
        let (sender, receiver) = mpsc::channel();
        let last_error = Arc::new(Mutex::new(None));
        let learning = LearningClient {
            sender,
            last_error: Arc::clone(&last_error),
        };
        let worker_snapshots = snapshots.clone();
        let worker = thread::spawn(move || {
            learning_worker(
                database,
                worker_snapshots,
                receiver,
                last_error,
                minimum_confidence,
            )
        });
        Ok(Self {
            snapshots,
            learning,
            minimum_confidence,
            worker: Some(worker),
        })
    }

    pub fn snapshots(&self) -> LexicalSnapshotStore {
        self.snapshots.clone()
    }

    pub fn learning(&self) -> LearningClient {
        self.learning.clone()
    }

    pub fn session(&self) -> AdaptiveCorrectionSession {
        AdaptiveCorrectionSession::new(self.snapshots(), self.learning(), self.minimum_confidence)
    }

    pub fn flush(&self) -> Result<(), AdaptiveRuntimeError> {
        self.learning.flush()
    }
}

impl Drop for AdaptiveLexicalRuntime {
    fn drop(&mut self) {
        let _ = self.learning.send(LearningCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[derive(Debug)]
struct PendingCorrection {
    observed_text: String,
    replacement_text: String,
    observed_at_ms: i64,
    action: ReplacementAction,
}

#[derive(Debug)]
struct UndoCandidate {
    receipt: CorrectionEventReceipt,
    action: ReplacementAction,
}

#[derive(Debug)]
struct PendingUndo {
    receipt: CorrectionEventReceipt,
    action: ReplacementAction,
    undone_at_ms: i64,
}

pub struct AdaptiveCorrectionSession {
    input: InputBuffer,
    snapshots: LexicalSnapshotStore,
    learning: LearningClient,
    minimum_confidence: Confidence,
    replacements: ReplacementEngine,
    pending_correction: Option<PendingCorrection>,
    undo_candidate: Option<UndoCandidate>,
    pending_undo: Option<PendingUndo>,
}

impl AdaptiveCorrectionSession {
    fn new(
        snapshots: LexicalSnapshotStore,
        learning: LearningClient,
        minimum_confidence: Confidence,
    ) -> Self {
        Self {
            input: InputBuffer::new(),
            snapshots,
            learning,
            minimum_confidence,
            replacements: ReplacementEngine::new(),
            pending_correction: None,
            undo_candidate: None,
            pending_undo: None,
        }
    }

    pub fn process(
        &mut self,
        event: InputEvent,
        observed_at_ms: i64,
    ) -> Result<AdaptiveCorrectionDirective, AdaptiveRuntimeError> {
        // A side-effect result is valid only for the directive that was returned immediately
        // before it. Any intervening observed input makes a previously planned correction or Undo
        // stale, including an Undo whose external text restoration is still in flight.
        self.pending_correction = None;
        self.undo_candidate = None;
        self.pending_undo = None;
        let InputOutcome::Completed(token) = self.input.process(event) else {
            return Ok(AdaptiveCorrectionDirective::Pass);
        };

        let snapshot = self.snapshots.load()?;
        let provider = LexicalCorrectionProvider::from_snapshot(snapshot);
        let decision = CorrectionEngine::new(provider, self.minimum_confidence).decide(&token);
        let action = self.replacements.plan(&token, decision.clone());
        match &decision {
            CorrectionDecision::Keep => {
                self.learning
                    .observe_typed_token(token.text(), observed_at_ms)?;
            }
            CorrectionDecision::Replace(replacement) => {
                if let Some(action) = action.as_ref() {
                    self.pending_correction = Some(PendingCorrection {
                        observed_text: token.text().to_owned(),
                        replacement_text: replacement.as_str().to_owned(),
                        observed_at_ms,
                        action: action.clone(),
                    });
                }
            }
        }

        Ok(match action {
            Some(action) => AdaptiveCorrectionDirective::Replace(action),
            None => AdaptiveCorrectionDirective::Pass,
        })
    }

    pub fn replacement_outcome(
        &mut self,
        outcome: ReplacementOutcome,
    ) -> Result<(), AdaptiveRuntimeError> {
        let Some(pending) = self.pending_correction.take() else {
            return Ok(());
        };
        if outcome == ReplacementOutcome::Applied {
            let receipt = self.learning.record_correction(
                pending.observed_text.clone(),
                pending.replacement_text,
                pending.observed_at_ms,
            )?;
            if let Some(action) = self
                .replacements
                .plan_immediate_undo(&pending.observed_text, &pending.action)
            {
                self.undo_candidate = Some(UndoCandidate { receipt, action });
            }
        }
        Ok(())
    }

    pub fn request_undo(
        &mut self,
        undone_at_ms: i64,
    ) -> Result<Option<ReplacementAction>, AdaptiveRuntimeError> {
        if self.pending_undo.is_some() {
            return Ok(None);
        }
        let Some(candidate) = self.undo_candidate.take() else {
            return Ok(None);
        };
        let action = candidate.action.clone();
        self.pending_undo = Some(PendingUndo {
            receipt: candidate.receipt,
            action: candidate.action,
            undone_at_ms,
        });
        Ok(Some(action))
    }

    pub fn undo_outcome(&mut self, outcome: UndoOutcome) -> Result<(), AdaptiveRuntimeError> {
        let Some(pending) = self.pending_undo.take() else {
            return Ok(());
        };
        match outcome {
            UndoOutcome::Applied => self
                .learning
                .queue_commit_recorded_undo(pending.receipt, pending.undone_at_ms),
            UndoOutcome::NotExecuted => {
                self.undo_candidate = Some(UndoCandidate {
                    receipt: pending.receipt,
                    action: pending.action,
                });
                Ok(())
            }
            UndoOutcome::Uncertain => Ok(()),
        }
    }
}

fn learning_worker(
    mut database: Database,
    snapshots: LexicalSnapshotStore,
    receiver: mpsc::Receiver<LearningCommand>,
    last_error: Arc<Mutex<Option<String>>>,
    minimum_confidence: Confidence,
) {
    while let Ok(command) = receiver.recv() {
        let result = match command {
            LearningCommand::ObserveToken { token, used_at_ms } => {
                observe_user_word(&database, &snapshots, &token, used_at_ms)
            }
            LearningCommand::ObserveText { text, used_at_ms } => {
                observe_user_text(&database, &snapshots, &text, used_at_ms, minimum_confidence)
            }
            LearningCommand::RecordCorrection {
                observed_text,
                replacement_text,
                created_at_ms,
                response,
            } => {
                let result = database
                    .record_correction_event(&observed_text, &replacement_text, created_at_ms)
                    .map_err(AdaptiveRuntimeError::from);
                let response_result = result.as_ref().copied().map_err(ToString::to_string);
                let _ = response.send(response_result);
                result.map(|_| ())
            }
            LearningCommand::PrepareUndo { event_id, response } => {
                let result = database
                    .prepare_correction_undo(event_id)
                    .map_err(|error| error.to_string());
                let _ = response.send(result);
                Ok(())
            }
            LearningCommand::CommitUndo {
                event_id,
                undone_at_ms,
                response,
            } => {
                let result = database
                    .commit_correction_undo(event_id, undone_at_ms)
                    .and_then(|_| database.load_user_lexicon())
                    .map_err(AdaptiveRuntimeError::from)
                    .and_then(|lexicon| snapshots.replace_user_lexicon(lexicon));
                let response_result = result.as_ref().map(|_| ()).map_err(ToString::to_string);
                let _ = response.send(response_result);
                result
            }
            LearningCommand::CommitRecordedUndo {
                receipt,
                undone_at_ms,
            } => match receipt.response.recv() {
                Ok(Ok(event_id)) => database
                    .commit_correction_undo(event_id, undone_at_ms)
                    .and_then(|_| database.load_user_lexicon())
                    .map_err(AdaptiveRuntimeError::from)
                    .and_then(|lexicon| snapshots.replace_user_lexicon(lexicon)),
                Ok(Err(error)) => Err(AdaptiveRuntimeError::LearningWorkerFailed(error)),
                Err(_) => Err(AdaptiveRuntimeError::LearningWorkerUnavailable),
            },
            LearningCommand::Barrier(response) => {
                let _ = response.send(());
                Ok(())
            }
            LearningCommand::Shutdown => break,
        };

        if let Err(error) = result
            && let Ok(mut slot) = last_error.lock()
        {
            *slot = Some(error.to_string());
        }
    }
}

fn observe_user_text(
    database: &Database,
    snapshots: &LexicalSnapshotStore,
    text: &str,
    used_at_ms: i64,
    minimum_confidence: Confidence,
) -> Result<(), AdaptiveRuntimeError> {
    let snapshot = snapshots.load()?;
    let provider = LexicalCorrectionProvider::from_snapshot(Arc::clone(&snapshot));
    let correction = CorrectionEngine::new(provider, minimum_confidence);
    let mut changed = false;
    let persist_result = (|| {
        for token in extract_tokens(text) {
            let completed = CompletedToken::new(token, Boundary::Character(' '));
            if correction.decide(&completed) == CorrectionDecision::Keep
                && should_record_user_word(token, &snapshot)
            {
                database.record_user_word(token, used_at_ms)?;
                changed = true;
            }
        }
        Ok(())
    })();

    // record_user_word writes canonical state before returning. If a later token fails, refresh
    // from the rows that did commit so the live snapshot never silently lags canonical SQLite.
    if changed {
        snapshots.replace_user_lexicon(database.load_user_lexicon()?)?;
    }
    persist_result
}

fn observe_user_word(
    database: &Database,
    snapshots: &LexicalSnapshotStore,
    token: &str,
    used_at_ms: i64,
) -> Result<(), AdaptiveRuntimeError> {
    let snapshot = snapshots.load()?;
    if !should_record_user_word(token, &snapshot) {
        return Ok(());
    }
    database.record_user_word(token, used_at_ms)?;
    snapshots.replace_user_lexicon(database.load_user_lexicon()?)
}

fn should_record_user_word(token: &str, snapshot: &LexicalSnapshot) -> bool {
    let normalized = normalize_word(token);
    if normalized.is_empty() {
        return false;
    }
    if snapshot.user_lexicon().contains_normalized(&normalized) {
        return true;
    }
    if snapshot
        .languages()
        .iter()
        .any(|language| language.contains_normalized(&normalized))
    {
        return false;
    }
    token.chars().any(char::is_alphabetic)
}

fn extract_tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split(|character: char| {
        !(character.is_alphanumeric() || matches!(character, '_' | '-' | '\'' | '’'))
    })
    .map(|token| token.trim_matches(['-', '_', '\'', '’']))
    .filter(|token| !token.is_empty())
}
