//! Stop the authenticated batch at its sink; never fabricate durable success.
use async_trait::async_trait;
use statechronicle::Event;
use statechronicle::executor::pipeline::DurableBatchSink;
use statechronicle::intent::validated::ValidatedIntent;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
pub struct RefuseSink(pub AtomicUsize);

#[async_trait]
impl DurableBatchSink for RefuseSink {
    async fn persist_batch(
        &self,
        intents: &[ValidatedIntent],
        events: &[Event],
    ) -> Result<(), String> {
        assert_eq!(intents.len(), 2);
        assert_eq!(events.len(), 2);
        for (intent, event) in intents.iter().zip(events) {
            assert_eq!(intent.intent.intent_id, event.intent_id);
            assert_eq!(intent.intent.tenant_id, event.tenant_id);
        }
        self.0.fetch_add(1, Ordering::Relaxed);
        Err(String::from("benchmark refuses persistence"))
    }
}
