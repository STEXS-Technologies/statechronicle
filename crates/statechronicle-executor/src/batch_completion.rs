//! Compatibility batch completion; never the canonical ledger transaction owner.
use super::{
    DurableBatchSink, Event, ExecutorError, ValidatedIntent, map_transaction_manager_error,
};
use statechronicle_ports::transaction_manager::{TransactionHandle, TransactionManagerError};
use std::{borrow::Cow, future::Future};

// Planning and sink routes have distinct authorities. The original wrappers
// never held a symbolic handle alongside a sink; represent that invariant.
pub(super) enum BatchCompletion<'sink> {
    Symbolic(Box<dyn TransactionHandle + Send>),
    Sink(&'sink dyn DurableBatchSink),
}

impl<'sink> BatchCompletion<'sink> {
    pub(super) async fn begin<F>(
        sink: Option<&'sink dyn DurableBatchSink>,
        start: impl FnOnce() -> F,
    ) -> Result<Self, ExecutorError>
    where
        F: Future<Output = Result<Box<dyn TransactionHandle + Send>, TransactionManagerError>>,
    {
        match sink {
            Some(sink) => Ok(Self::Sink(sink)),
            None => start()
                .await
                .map(Self::Symbolic)
                .map_err(|error| map_transaction_manager_error(&error)),
        }
    }

    pub(super) async fn finish<T>(
        self,
        result: Result<T, ExecutorError>,
        intents: &[ValidatedIntent],
        events: impl for<'value> FnOnce(&'value T) -> Cow<'value, [Event]>,
        rollback_context: &'static str,
    ) -> Result<T, ExecutorError> {
        match result {
            Ok(value) => {
                // Group coordinators historically flatten even on the planning
                // route. Preserve that allocation/clone order through commit.
                let events = events(&value);
                match self {
                    Self::Sink(sink) if !events.is_empty() => sink
                        .persist_batch(intents, &events)
                        .await
                        .map_err(ExecutorError::Store)?,
                    Self::Symbolic(handle) => handle
                        .commit()
                        .await
                        .map_err(|error| map_transaction_manager_error(&error))?,
                    Self::Sink(_) => {}
                }
                drop(events);
                Ok(value)
            }
            Err(error) => {
                let message = error.to_string();
                if let Self::Symbolic(handle) = self
                    && let Err(rollback_error) = handle.rollback().await
                {
                    tracing::warn!(rollback = %rollback_error, "{rollback_context}");
                }
                Err(ExecutorError::AtomicityViolation(message))
            }
        }
    }
}
