//! Rebuild one tenant's projections from its verified canonical event stream.
//!
//! Usage:
//! `cargo run -p statechronicle-sqlite --example rebuild_projections -- <db> <tenant> <checkpoint-key> <chunk-size>`

use std::error::Error;

use statechronicle_domain::tenant::TenantId;
use statechronicle_sqlite::SqliteLedgerStore;

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or("missing database path")?;
    let tenant = TenantId(args.next().ok_or("missing tenant")?);
    let checkpoint_key = args.next().ok_or("missing checkpoint key")?;
    let chunk_size: usize = args
        .next()
        .ok_or("missing chunk size")?
        .parse()
        .map_err(|error| format!("chunk size must be a positive integer: {error}"))?;
    if args.next().is_some() || checkpoint_key.is_empty() || chunk_size == 0 {
        return Err(
            "usage: rebuild_projections <db> <tenant> <checkpoint-key> <chunk-size>".into(),
        );
    }

    let store = SqliteLedgerStore::open(path)?;
    store.verify_integrity(&tenant)?;
    let before = store.projection_rebuild_progress(&tenant, &checkpoint_key)?;
    println!(
        "before tenant={} total_events={} next_event={} lag_events={}",
        tenant.0, before.total_events, before.next_event, before.lag_events
    );

    let rebuilt = tokio::runtime::Runtime::new()?
        .block_on(store.rebuild_projections_from_canonical(&tenant, &checkpoint_key, chunk_size))?;
    let after = store.projection_rebuild_progress(&tenant, &checkpoint_key)?;
    if !after.caught_up {
        return Err("projection rebuild did not reach the canonical stream".into());
    }
    store.verify_integrity(&tenant)?;
    println!(
        "rebuilt tenant={} projections_written={} total_events={} next_event={} lag_events=0",
        tenant.0, rebuilt, after.total_events, after.next_event
    );
    store.clear_rebuild_checkpoint_for_tenant(&tenant, &checkpoint_key)?;
    Ok(())
}
