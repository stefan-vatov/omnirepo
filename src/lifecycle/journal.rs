//! Bounded concurrent producers to one ordered journal writer.
//!
//! Fleet workers submit typed events through a bounded channel; one writer
//! thread owns the run-record file, allocates strictly monotonic checkpoints,
//! validates transitions, appends exact JSONL lines, and acknowledges each
//! checkpoint only after the line is durably written.  Producers must not
//! start an external effect until their checkpoint is acknowledged.  A
//! persistence failure poisons the journal: every later submit fails and no
//! new effect can claim an acknowledgement.

#![allow(dead_code)]

use super::event::{
    Checkpoint, EventError, EventLog, INVOCATION_CHECKPOINT, JournalEvent, RunStage,
};
use super::run_record::RunRecord;
use std::{
    error::Error,
    fmt,
    sync::{
        Arc, Mutex,
        mpsc::{SyncSender, TrySendError, sync_channel},
    },
    thread::{self, JoinHandle},
};

#[cfg(test)]
mod journal_tests;

/// Default bounded producer queue: at most this many pending events before a
/// producer blocks (bounded backpressure) or observes a full channel.
pub const DEFAULT_QUEUE_CAPACITY: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JournalConfig {
    /// Maximum pending (unacknowledged) events queued for the writer.
    pub queue_capacity: usize,
    /// When true every append is fsync'd before its acknowledgement; when
    /// false the writer relies on the next append or shutdown sync.
    pub sync_each_append: bool,
}

impl Default for JournalConfig {
    fn default() -> Self {
        Self {
            queue_capacity: DEFAULT_QUEUE_CAPACITY,
            sync_each_append: true,
        }
    }
}

enum Message {
    Append {
        event: JournalEvent,
        ack: SyncSender<Result<Checkpoint, JournalError>>,
    },
    Shutdown,
}

/// Producer-side journal handle.  Cloning shares the same writer.
#[derive(Clone)]
pub struct JournalHandle {
    sender: SyncSender<Message>,
    poisoned: Arc<Mutex<bool>>,
}

impl JournalHandle {
    /// Submit one event.  Blocks while the bounded queue is full (bounded
    /// backpressure) and returns the acknowledged checkpoint only after the
    /// line is durably appended.  A poisoned writer fails every submit.
    pub fn submit(&self, event: JournalEvent) -> Result<Checkpoint, JournalError> {
        if *self.poisoned.lock().expect("poison flag") {
            return Err(JournalError::Poisoned);
        }
        let (ack, received) = sync_channel(1);
        self.sender
            .send(Message::Append { event, ack })
            .map_err(|_| JournalError::Poisoned)?;
        received.recv().map_err(|_| JournalError::Poisoned)?
    }

    /// Non-blocking submit.  Returns `Full` when the bounded queue is at
    /// capacity, so callers can apply their backpressure policy without
    /// blocking the caller's scheduling loop.
    pub fn try_submit(&self, event: JournalEvent) -> Result<Checkpoint, TrySubmitError> {
        if *self.poisoned.lock().expect("poison flag") {
            return Err(TrySubmitError::Poisoned);
        }
        let (ack, received) = sync_channel(1);
        match self.sender.try_send(Message::Append { event, ack }) {
            Ok(()) => received
                .recv()
                .map_err(|_| TrySubmitError::Poisoned)
                .and_then(|result| result.map_err(TrySubmitError::from)),
            Err(TrySendError::Full(_)) => Err(TrySubmitError::Full),
            Err(TrySendError::Disconnected(_)) => Err(TrySubmitError::Poisoned),
        }
    }
}

impl fmt::Debug for JournalHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("JournalHandle")
    }
}

/// Owning journal: the writer thread and its producer handle.
pub struct Journal {
    pub handle: JournalHandle,
    thread: Option<JoinHandle<()>>,
}

impl Journal {
    /// Start the single writer over the run record.  The invocation intent
    /// line is already checkpoint 0 in the record; the writer seeds its
    /// transition state with it, so the first appended event is checkpoint 1.
    pub fn start(record: RunRecord, config: JournalConfig) -> Self {
        let (sender, receiver) = sync_channel(config.queue_capacity);
        let poisoned = Arc::new(Mutex::new(false));
        let handle = JournalHandle {
            sender,
            poisoned: Arc::clone(&poisoned),
        };
        let mut log = EventLog::new();
        let run_id = record.id().to_string();
        let invocation = JournalEvent::RunIntent {
            checkpoint: INVOCATION_CHECKPOINT,
            run_id,
            stage: RunStage::Invocation,
        };
        log.record(&invocation)
            .expect("invocation intent is the first valid event");
        let thread = thread::spawn(move || {
            let mut record = record;
            let mut checkpoint: Checkpoint = 0;
            for message in receiver {
                match message {
                    Message::Append { event, ack } => {
                        let result = append_one(
                            &mut record,
                            &mut log,
                            &mut checkpoint,
                            config.sync_each_append,
                            event,
                        );
                        // Only persistence failures poison the journal;
                        // protocol rejections (invalid transitions, checkpoint
                        // mismatches) fail the submit and keep the writer
                        // available for valid events.
                        if matches!(result, Err(JournalError::Write(_))) {
                            *poisoned.lock().expect("poison flag") = true;
                        }
                        let _ = ack.send(result);
                        if *poisoned.lock().expect("poison flag") {
                            // Drain without accepting further effects.
                            break;
                        }
                    }
                    Message::Shutdown => break,
                }
            }
            if config.sync_each_append {
                let _ = record.sync_tail();
            }
        });
        Self {
            handle,
            thread: Some(thread),
        }
    }

    /// Stop the writer, syncing any buffered tail, and join the thread.
    pub fn shutdown(&mut self) -> Result<(), JournalError> {
        let _ = self.handle.sender.send(Message::Shutdown);
        self.thread
            .take()
            .expect("journal thread")
            .join()
            .map_err(|_| JournalError::Poisoned)
    }
}

fn append_one(
    record: &mut RunRecord,
    log: &mut EventLog,
    checkpoint: &mut Checkpoint,
    sync_each: bool,
    event: JournalEvent,
) -> Result<Checkpoint, JournalError> {
    *checkpoint += 1;
    // The writer owns checkpoint allocation; the producer's declared value
    // is never trusted.
    let assigned = event.with_checkpoint(*checkpoint);
    log.record(&assigned).map_err(JournalError::Invalid)?;
    let line = assigned.render();
    record
        .append(line.as_bytes())
        .map_err(|error| JournalError::Write(error.to_string()))?;
    if sync_each {
        record
            .sync_tail()
            .map_err(|error| JournalError::Write(error.to_string()))?;
    }
    Ok(*checkpoint)
}

/// Journal persistence and protocol failures.
#[derive(Debug)]
pub enum JournalError {
    /// A write or sync failure; the journal is poisoned.
    Write(String),
    /// The event violates the transition contract.
    Invalid(EventError),
    /// A prior persistence failure; no new effects may be acknowledged.
    Poisoned,
}

impl fmt::Display for JournalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Write(reason) => write!(formatter, "journal write failed: {reason}"),
            Self::Invalid(error) => write!(formatter, "journal rejected event: {error}"),
            Self::Poisoned => formatter.write_str("journal writer is poisoned"),
        }
    }
}
impl Error for JournalError {}

/// Non-blocking submit outcome.
#[derive(Debug)]
pub enum TrySubmitError {
    /// The bounded queue is full; apply the caller's backpressure policy.
    Full,
    /// The writer is poisoned or gone.
    Poisoned,
    /// The writer rejected the event or the acknowledgement failed.
    Rejected(JournalError),
}

impl From<JournalError> for TrySubmitError {
    fn from(error: JournalError) -> Self {
        Self::Rejected(error)
    }
}

impl fmt::Display for TrySubmitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full => formatter.write_str("journal queue is full"),
            Self::Poisoned => formatter.write_str("journal writer is poisoned"),
            Self::Rejected(error) => write!(formatter, "journal rejected event: {error}"),
        }
    }
}
impl Error for TrySubmitError {}
