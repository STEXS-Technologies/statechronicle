//! Verify every tenant in a file-backed StateChronicle SQLite database.

use std::error::Error;

use statechronicle_sqlite::SqliteLedgerStore;

fn main() -> Result<(), Box<dyn Error>> {
    let path = std::env::args().nth(1).ok_or(
        "usage: cargo run -p statechronicle-sqlite --example verify_integrity -- <db-path>",
    )?;
    let store = SqliteLedgerStore::open(path)?;
    let tenants = store.tenants()?;
    let mut failed = false;
    for tenant in tenants {
        match store.verify_integrity(&tenant) {
            Ok(report) => println!(
                "tenant={} commits={} events={} head={}",
                report.tenant.0,
                report.commit_count,
                report.event_count,
                report
                    .head_commit_id
                    .map_or_else(|| String::from("<genesis>"), |id| id.0)
            ),
            Err(error) => {
                eprintln!("tenant={} integrity failure: {error}", tenant.0);
                failed = true;
            }
        }
    }
    if failed {
        return Err("one or more tenant integrity checks failed".into());
    }
    Ok(())
}
