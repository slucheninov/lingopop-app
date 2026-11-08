//! Load and persist rules shared by `HistoryStore` and `UsageStore`, implemented once.

use std::path::PathBuf;
use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::envelope::{Mergeable, MergeableEnvelope};
use crate::fsio::{atomic_write, is_blank, quarantine, read_optional, LoadOutcome, StoreError};
use crate::time::{Clock, Timestamp};

/// A store file (`history.json` / `usage.json`) and its in-memory envelope.
pub(crate) struct EnvelopeFile<T> {
    path: PathBuf,
    clock: Arc<dyn Clock>,
    envelope: MergeableEnvelope<T>,
}

impl<T: Mergeable + Serialize + DeserializeOwned + Clone> EnvelopeFile<T> {
    /// Reads the file; never writes. Missing / blank = empty, legacy bare array and
    /// BOM are accepted, an undecodable file is moved aside with `quarantine` and the
    /// store starts empty (the bad bytes are never overwritten).
    pub(crate) fn load(
        path: PathBuf,
        clock: Arc<dyn Clock>,
    ) -> Result<(Self, LoadOutcome), StoreError> {
        Self::load_bytes(path, clock, None)
    }

    pub(crate) fn load_bounded(
        path: PathBuf,
        clock: Arc<dyn Clock>,
        max_bytes: u64,
    ) -> Result<(Self, LoadOutcome), StoreError> {
        Self::load_bytes(path, clock, Some(max_bytes))
    }

    fn load_bytes(
        path: PathBuf,
        clock: Arc<dyn Clock>,
        max_bytes: Option<u64>,
    ) -> Result<(Self, LoadOutcome), StoreError> {
        let bytes = if let Some(limit) = max_bytes {
            use std::io::Read;
            match std::fs::File::open(&path) {
                Ok(file) => {
                    let mut bytes = Vec::new();
                    file.take(limit + 1).read_to_end(&mut bytes)?;
                    if bytes.len() as u64 > limit {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "store file exceeds size limit",
                        )
                        .into());
                    }
                    Some(bytes)
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(error.into()),
            }
        } else {
            read_optional(&path)?
        };
        let (envelope, outcome) = match bytes {
            None => (MergeableEnvelope::new(), LoadOutcome::Missing),
            Some(bytes) if is_blank(&bytes) => (MergeableEnvelope::new(), LoadOutcome::Missing),
            Some(bytes) => match MergeableEnvelope::<T>::decode(&bytes) {
                Some(decoded) if decoded.is_legacy => (decoded.envelope, LoadOutcome::Legacy),
                Some(decoded) => (decoded.envelope, LoadOutcome::Loaded),
                None => {
                    let backup = quarantine(&path, clock.now())?;
                    (
                        MergeableEnvelope::new(),
                        LoadOutcome::CorruptBackedUp(backup),
                    )
                }
            },
        };
        Ok((
            Self {
                path,
                clock,
                envelope,
            },
            outcome,
        ))
    }

    pub(crate) fn envelope(&self) -> &MergeableEnvelope<T> {
        &self.envelope
    }

    pub(crate) fn now(&self) -> Timestamp {
        self.clock.now()
    }

    /// In-memory normalization on open (sorting, retention); nothing is written.
    pub(crate) fn adjust_in_memory(&mut self, adjust: impl FnOnce(&mut MergeableEnvelope<T>)) {
        adjust(&mut self.envelope);
    }

    /// Prunes tombstones older than 90 days, writes the result atomically and only
    /// then swaps it into memory, so a failed write leaves the state unchanged.
    pub(crate) fn persist(&mut self, next: MergeableEnvelope<T>) -> Result<(), StoreError> {
        let pruned = next.pruned(self.clock.now());
        let bytes = pruned.encode()?;
        atomic_write(&self.path, &bytes, true)?;
        self.envelope = pruned;
        Ok(())
    }
}
