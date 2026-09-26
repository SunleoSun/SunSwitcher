use std::error::Error;
use std::fmt::{Display, Formatter};
use std::sync::{Arc, Mutex, RwLock, mpsc};
use std::thread::{self, JoinHandle};

use crate::completion::sequence::{MAX_SEQUENCE_WORDS, SequenceHistory};
use crate::completion::{
    CompletionApplyOutcome, CompletionCommand, CompletionCommandResult, CompletionDeletionTarget,
    CompletionProvider, CompletionSession, CompletionSuggestion,
    CompletionSuppressionSnapshotStore, SequenceSnapshotStore,
};
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
    ObserveTypedSequence {
        canonical_tokens: Vec<String>,
        used_at_ms: i64,
    },
    DeleteCompletion {
        target: CompletionDeletionTarget,
    },
    DeleteUserWord {
        term: String,
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

    fn observe_typed_sequence(
        &self,
        canonical_tokens: Vec<String>,
        used_at_ms: i64,
    ) -> Result<(), AdaptiveRuntimeError> {
        self.send(LearningCommand::ObserveTypedSequence {
            canonical_tokens,
            used_at_ms,
        })
    }

    fn delete_completion(
        &self,
        target: CompletionDeletionTarget,
    ) -> Result<(), AdaptiveRuntimeError> {
        self.send(LearningCommand::DeleteCompletion { target })
    }

    pub fn delete_user_word(&self, term: impl Into<String>) -> Result<(), AdaptiveRuntimeError> {
        self.send(LearningCommand::DeleteUserWord { term: term.into() })
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
    sequences: SequenceSnapshotStore,
    completion_suppressions: CompletionSuppressionSnapshotStore,
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
        let sequence_history = database.load_text_history()?;
        let completion_suppressions = database.load_hidden_completion_words()?;
        let snapshot = LexicalSnapshot::try_new(languages, user_lexicon)
            .map_err(|_| AdaptiveRuntimeError::NoLanguages)?;
        let snapshots = LexicalSnapshotStore::new(snapshot);
        let sequences = SequenceSnapshotStore::new(sequence_history);
        let completion_suppressions =
            CompletionSuppressionSnapshotStore::new(completion_suppressions);
        let (sender, receiver) = mpsc::channel();
        let last_error = Arc::new(Mutex::new(None));
        let learning = LearningClient {
            sender,
            last_error: Arc::clone(&last_error),
        };
        let worker_snapshots = snapshots.clone();
        let worker_sequences = sequences.clone();
        let worker_completion_suppressions = completion_suppressions.clone();
        let worker = thread::spawn(move || {
            learning_worker(
                database,
                worker_snapshots,
                worker_sequences,
                worker_completion_suppressions,
                receiver,
                last_error,
                minimum_confidence,
            )
        });
        Ok(Self {
            snapshots,
            sequences,
            completion_suppressions,
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

    pub fn completion_provider(&self) -> Result<CompletionProvider, AdaptiveRuntimeError> {
        let lexical = self.snapshots.load()?;
        let sequences = self
            .sequences
            .load()
            .map_err(|_| AdaptiveRuntimeError::SnapshotUnavailable)?;
        let completion_suppressions = self
            .completion_suppressions
            .load()
            .map_err(|_| AdaptiveRuntimeError::SnapshotUnavailable)?;
        Ok(CompletionProvider::with_word_suppressions(
            lexical,
            sequences,
            completion_suppressions,
        ))
    }

    pub fn session(&self) -> AdaptiveCorrectionSession {
        AdaptiveCorrectionSession::new(self.snapshots(), self.learning(), self.minimum_confidence)
    }

    pub fn completion_session(&self) -> AdaptiveCompletionSession {
        AdaptiveCompletionSession::new(
            self.snapshots.clone(),
            self.sequences.clone(),
            self.completion_suppressions.clone(),
            self.learning(),
        )
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

pub struct AdaptiveCompletionSession {
    lexical: LexicalSnapshotStore,
    sequences: SequenceSnapshotStore,
    completion_suppressions: CompletionSuppressionSnapshotStore,
    learning: LearningClient,
    session: CompletionSession,
    pending_accepted_words: Vec<String>,
}

impl AdaptiveCompletionSession {
    fn new(
        lexical: LexicalSnapshotStore,
        sequences: SequenceSnapshotStore,
        completion_suppressions: CompletionSuppressionSnapshotStore,
        learning: LearningClient,
    ) -> Self {
        Self {
            lexical,
            sequences,
            completion_suppressions,
            learning,
            session: CompletionSession::default(),
            pending_accepted_words: Vec::new(),
        }
    }

    pub fn process_event(
        &mut self,
        event: InputEvent,
        observed_at_ms: i64,
    ) -> Result<(), AdaptiveRuntimeError> {
        self.pending_accepted_words.clear();
        self.session.process_event(event);
        let provider = CompletionProvider::with_word_suppressions(
            self.lexical.load()?,
            self.sequences
                .load()
                .map_err(|_| AdaptiveRuntimeError::SnapshotUnavailable)?,
            self.completion_suppressions
                .load()
                .map_err(|_| AdaptiveRuntimeError::SnapshotUnavailable)?,
        );
        self.session.refresh(&provider, observed_at_ms);
        Ok(())
    }

    pub fn canonicalize_last_context_word(&mut self, canonical: &str) {
        self.session.canonicalize_last_context_word(canonical);
    }

    pub fn command(
        &mut self,
        command: CompletionCommand,
    ) -> Result<CompletionCommandResult, AdaptiveRuntimeError> {
        let accepted_words = self.selected_acceptance_words(command);
        let result = self.session.command(command);
        if matches!(
            result,
            CompletionCommandResult::AcceptSuffix(_) | CompletionCommandResult::AcceptWord(_)
        ) {
            self.pending_accepted_words = accepted_words;
        } else {
            self.pending_accepted_words.clear();
        }
        if let CompletionCommandResult::DeletePrediction(target) = &result {
            self.learning.delete_completion(target.clone())?;
            return Ok(CompletionCommandResult::Consumed);
        }
        Ok(result)
    }

    pub fn suffix_acceptance_outcome(
        &mut self,
        outcome: CompletionApplyOutcome,
        observed_at_ms: i64,
    ) -> Result<(), AdaptiveRuntimeError> {
        self.learn_pending_acceptance(outcome, observed_at_ms)
    }

    pub fn word_acceptance_outcome(
        &mut self,
        outcome: CompletionApplyOutcome,
        observed_at_ms: i64,
    ) -> Result<(), AdaptiveRuntimeError> {
        self.session.word_acceptance_outcome(outcome);
        self.learn_pending_acceptance(outcome, observed_at_ms)?;
        if outcome != CompletionApplyOutcome::Applied {
            return Ok(());
        }

        let provider = CompletionProvider::with_word_suppressions(
            self.lexical.load()?,
            self.sequences
                .load()
                .map_err(|_| AdaptiveRuntimeError::SnapshotUnavailable)?,
            self.completion_suppressions
                .load()
                .map_err(|_| AdaptiveRuntimeError::SnapshotUnavailable)?,
        );
        self.session.refresh(&provider, observed_at_ms);
        Ok(())
    }

    fn selected_acceptance_words(&self, command: CompletionCommand) -> Vec<String> {
        let Some(suggestion) = self
            .session
            .suggestions()
            .get(self.session.selected_index())
        else {
            return Vec::new();
        };
        match command {
            CompletionCommand::Accept => suggestion
                .text()
                .split_whitespace()
                .map(str::to_owned)
                .collect(),
            CompletionCommand::AcceptNextWord => suggestion
                .text()
                .split_whitespace()
                .next()
                .map(str::to_owned)
                .into_iter()
                .collect(),
            _ => Vec::new(),
        }
    }

    fn learn_pending_acceptance(
        &mut self,
        outcome: CompletionApplyOutcome,
        observed_at_ms: i64,
    ) -> Result<(), AdaptiveRuntimeError> {
        let accepted_words = std::mem::take(&mut self.pending_accepted_words);
        if outcome != CompletionApplyOutcome::Applied {
            return Ok(());
        }
        for word in accepted_words {
            self.learning.observe_typed_token(word, observed_at_ms)?;
        }
        Ok(())
    }

    pub fn suggestions(&self) -> &[CompletionSuggestion] {
        self.session.suggestions()
    }

    pub fn selected_index(&self) -> usize {
        self.session.selected_index()
    }

    pub fn is_active(&self) -> bool {
        self.session.is_active()
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

#[derive(Debug)]
struct PendingTypedSequence {
    canonical_tokens: Vec<String>,
    boundary: Boundary,
    observed_at_ms: i64,
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
    typed_sequence_context: Vec<String>,
    pending_typed_sequence: Option<PendingTypedSequence>,
    resolved_completion_word: Option<String>,
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
            typed_sequence_context: Vec::new(),
            pending_typed_sequence: None,
            resolved_completion_word: None,
        }
    }

    pub fn process(
        &mut self,
        event: InputEvent,
        observed_at_ms: i64,
    ) -> Result<AdaptiveCorrectionDirective, AdaptiveRuntimeError> {
        self.resolved_completion_word = None;
        // A side-effect result is valid only for the directive returned immediately before it.
        // If a correction/Undo side effect is still in flight when new input arrives, visible text
        // is no longer provable, so the typed sequence context is discarded instead of guessed.
        if self.pending_correction.is_some() || self.pending_undo.is_some() {
            self.pending_typed_sequence = None;
            self.typed_sequence_context.clear();
        } else {
            self.finalize_pending_typed_sequence()?;
        }
        self.pending_correction = None;
        self.undo_candidate = None;
        self.pending_undo = None;

        let outcome = self.input.process(event);
        let token = match outcome {
            InputOutcome::Continue => return Ok(AdaptiveCorrectionDirective::Pass),
            InputOutcome::Invalidated => {
                self.typed_sequence_context.clear();
                return Ok(AdaptiveCorrectionDirective::Pass);
            }
            InputOutcome::Completed(token) => token,
        };

        let snapshot = self.snapshots.load()?;
        let provider = LexicalCorrectionProvider::from_snapshot(snapshot);
        let canonical_kept_term = provider.canonical_learning_term(&token);
        let decision = CorrectionEngine::new(provider, self.minimum_confidence).decide(&token);
        let action = self.replacements.plan(&token, decision.clone());
        match &decision {
            CorrectionDecision::Keep => {
                self.resolved_completion_word = Some(canonical_kept_term.clone());
                self.learning
                    .observe_typed_token(&canonical_kept_term, observed_at_ms)?;
                let canonical_tokens =
                    self.push_typed_sequence_word(&canonical_kept_term, token.boundary());
                self.learning
                    .observe_typed_sequence(canonical_tokens, observed_at_ms)?;
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

        match outcome {
            ReplacementOutcome::Applied => {
                let receipt = self.learning.record_correction(
                    pending.observed_text.clone(),
                    pending.replacement_text.clone(),
                    pending.observed_at_ms,
                )?;
                let canonical_tokens = if let Some(canonical_replacement) =
                    canonical_sequence_word(&pending.replacement_text)
                {
                    self.resolved_completion_word = Some(canonical_replacement.clone());
                    self.push_typed_sequence_word(&canonical_replacement, pending.action.boundary())
                } else {
                    self.resolved_completion_word = Some(String::new());
                    self.pending_typed_sequence = None;
                    self.typed_sequence_context.clear();
                    Vec::new()
                };
                if let Some(action) = self
                    .replacements
                    .plan_immediate_undo(&pending.observed_text, &pending.action)
                {
                    self.undo_candidate = Some(UndoCandidate { receipt, action });
                    if !canonical_tokens.is_empty() {
                        self.pending_typed_sequence = Some(PendingTypedSequence {
                            canonical_tokens,
                            boundary: pending.action.boundary(),
                            observed_at_ms: pending.observed_at_ms,
                        });
                    }
                } else if !canonical_tokens.is_empty() {
                    self.learning
                        .observe_typed_sequence(canonical_tokens, pending.observed_at_ms)?;
                }
            }
            ReplacementOutcome::Aborted => {
                self.resolved_completion_word = None;
                self.pending_typed_sequence = None;
                self.typed_sequence_context.clear();
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
            UndoOutcome::Applied => {
                let original_text = pending.action.replacement().as_str();
                self.learning
                    .queue_commit_recorded_undo(pending.receipt, pending.undone_at_ms)?;

                if let Some(mut typed) = self.pending_typed_sequence.take() {
                    let Some(canonical_original) = canonical_sequence_word(original_text) else {
                        self.typed_sequence_context.clear();
                        return Ok(());
                    };
                    self.resolved_completion_word = Some(canonical_original.clone());
                    if let Some(last) = typed.canonical_tokens.last_mut() {
                        *last = canonical_original.clone();
                    }
                    if matches!(typed.boundary, Boundary::Character(' '))
                        && let Some(last) = self.typed_sequence_context.last_mut()
                    {
                        *last = canonical_original;
                    }
                    self.learning
                        .observe_typed_sequence(typed.canonical_tokens, pending.undone_at_ms)?;
                }
                Ok(())
            }
            UndoOutcome::NotExecuted => {
                self.undo_candidate = Some(UndoCandidate {
                    receipt: pending.receipt,
                    action: pending.action,
                });
                Ok(())
            }
            UndoOutcome::Uncertain => {
                self.pending_typed_sequence = None;
                self.typed_sequence_context.clear();
                Ok(())
            }
        }
    }

    pub fn current_layout_switch_span(&self) -> &str {
        self.input.current_layout_span()
    }

    pub fn tracked_layout_switch_applied(&mut self, replacement: &str) {
        self.pending_correction = None;
        self.undo_candidate = None;
        self.pending_undo = None;
        self.pending_typed_sequence = None;
        self.typed_sequence_context.clear();
        self.resolved_completion_word = None;
        self.input.replace_layout_switch_span(replacement);
    }

    pub fn take_resolved_completion_word(&mut self) -> Option<String> {
        self.resolved_completion_word.take()
    }

    fn finalize_pending_typed_sequence(&mut self) -> Result<(), AdaptiveRuntimeError> {
        let Some(pending) = self.pending_typed_sequence.as_ref() else {
            return Ok(());
        };
        self.learning
            .observe_typed_sequence(pending.canonical_tokens.clone(), pending.observed_at_ms)?;
        self.pending_typed_sequence = None;
        Ok(())
    }

    fn push_typed_sequence_word(&mut self, token: &str, boundary: Boundary) -> Vec<String> {
        if token.is_empty() {
            self.typed_sequence_context.clear();
            return Vec::new();
        }
        self.typed_sequence_context.push(token.to_owned());
        if self.typed_sequence_context.len() > MAX_SEQUENCE_WORDS {
            let excess = self.typed_sequence_context.len() - MAX_SEQUENCE_WORDS;
            self.typed_sequence_context.drain(..excess);
        }
        let canonical_tokens = self.typed_sequence_context.clone();
        if !matches!(boundary, Boundary::Character(' ')) {
            self.typed_sequence_context.clear();
        }
        canonical_tokens
    }
}

fn learning_worker(
    mut database: Database,
    snapshots: LexicalSnapshotStore,
    sequences: SequenceSnapshotStore,
    completion_suppressions: CompletionSuppressionSnapshotStore,
    receiver: mpsc::Receiver<LearningCommand>,
    last_error: Arc<Mutex<Option<String>>>,
    minimum_confidence: Confidence,
) {
    while let Ok(command) = receiver.recv() {
        let result = match command {
            LearningCommand::ObserveToken { token, used_at_ms } => {
                observe_user_word(&database, &snapshots, &token, used_at_ms)
            }
            LearningCommand::ObserveText { text, used_at_ms } => observe_user_text(
                &database,
                &snapshots,
                &sequences,
                &text,
                used_at_ms,
                minimum_confidence,
            ),
            LearningCommand::ObserveTypedSequence {
                canonical_tokens,
                used_at_ms,
            } => observe_typed_sequence(&database, &sequences, &canonical_tokens, used_at_ms),
            LearningCommand::DeleteUserWord { term } => database
                .delete_user_word(&term)
                .map_err(AdaptiveRuntimeError::from)
                .and_then(|deleted| {
                    if deleted {
                        snapshots.replace_user_lexicon(database.load_user_lexicon()?)
                    } else {
                        Ok(())
                    }
                }),
            LearningCommand::DeleteCompletion { target } => match target {
                CompletionDeletionTarget::TextHistory(text) => database
                    .delete_text_history(&text)
                    .map_err(AdaptiveRuntimeError::from)
                    .and_then(|_| {
                        sequences
                            .replace(database.load_text_history()?)
                            .map_err(|_| AdaptiveRuntimeError::SnapshotUnavailable)
                    }),
                CompletionDeletionTarget::Word(normalized_term) => database
                    .hide_completion_word(&normalized_term)
                    .map_err(AdaptiveRuntimeError::from)
                    .and_then(|_| {
                        completion_suppressions
                            .replace(database.load_hidden_completion_words()?)
                            .map_err(|_| AdaptiveRuntimeError::SnapshotUnavailable)
                    }),
            },
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
    sequences: &SequenceSnapshotStore,
    text: &str,
    used_at_ms: i64,
    minimum_confidence: Confidence,
) -> Result<(), AdaptiveRuntimeError> {
    let mut snapshot = snapshots.load()?;
    let mut canonical_tokens = Vec::new();

    for token in extract_tokens(text) {
        let provider = LexicalCorrectionProvider::from_snapshot(Arc::clone(&snapshot));
        let correction = CorrectionEngine::new(provider, minimum_confidence);
        let completed = CompletedToken::new(token, Boundary::Character(' '));
        let decision = correction.decide(&completed);
        let canonical = match &decision {
            CorrectionDecision::Keep => token.to_owned(),
            CorrectionDecision::Replace(replacement) => replacement.as_str().to_owned(),
        };
        canonical_tokens.push(canonical);

        if decision != CorrectionDecision::Keep || !should_record_user_word(token, &snapshot) {
            continue;
        }

        database.record_user_word(token, used_at_ms)?;
        let user_lexicon = database.load_user_lexicon()?;
        snapshots.replace_user_lexicon(user_lexicon)?;
        snapshot = snapshots.load()?;
    }

    persist_sequence_observations(
        database,
        sequences,
        SequenceHistory::ngrams_from_canonical_tokens(&canonical_tokens),
        used_at_ms,
    )
}

fn observe_typed_sequence(
    database: &Database,
    sequences: &SequenceSnapshotStore,
    canonical_tokens: &[String],
    used_at_ms: i64,
) -> Result<(), AdaptiveRuntimeError> {
    persist_sequence_observations(
        database,
        sequences,
        SequenceHistory::suffix_ngrams_from_canonical_tokens(canonical_tokens),
        used_at_ms,
    )
}

fn persist_sequence_observations(
    database: &Database,
    sequences: &SequenceSnapshotStore,
    observed_sequences: Vec<String>,
    used_at_ms: i64,
) -> Result<(), AdaptiveRuntimeError> {
    if observed_sequences.is_empty() {
        return Ok(());
    }

    let mut changed = false;
    let persist_result = (|| {
        for sequence in observed_sequences {
            database.record_text_history(&sequence, used_at_ms)?;
            changed = true;
        }
        Ok(())
    })();

    if changed {
        sequences
            .replace(database.load_text_history()?)
            .map_err(|_| AdaptiveRuntimeError::SnapshotUnavailable)?;
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

fn canonical_sequence_word(text: &str) -> Option<String> {
    let mut tokens = extract_tokens(text);
    let token = tokens.next()?.to_owned();
    tokens.next().is_none().then_some(token)
}

fn extract_tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split(|character: char| {
        !(character.is_alphanumeric() || matches!(character, '_' | '-' | '\'' | '’'))
    })
    .map(|token| token.trim_matches(['-', '_', '\'', '’']))
    .filter(|token| !token.is_empty())
}
