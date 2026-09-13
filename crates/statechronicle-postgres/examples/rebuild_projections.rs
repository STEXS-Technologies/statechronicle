//! Rebuild one PostgreSQL tenant's projections from its verified event stream.
//!
//! Usage:
//! `STATECHRONICLE_POSTGRES_URL='...' cargo run -p statechronicle-postgres --example rebuild_projections -- <tenant> [--keep-existing]`

use statechronicle_domain::tenant::TenantId;
use statechronicle_postgres::PostgresLedgerStore;

#[tokio::main]
async fn main() {
    let url = match std::env::var("STATECHRONICLE_POSTGRES_URL") {
        Ok(url) if !url.trim().is_empty() => url,
        _ => {
            eprintln!("STATECHRONICLE_POSTGRES_URL must be set");
            std::process::exit(2);
        }
    };
    let mut arguments = std::env::args().skip(1);
    let tenant = match arguments.next() {
        Some(value) if !value.is_empty() => TenantId(value),
        _ => {
            eprintln!("usage: rebuild_projections <tenant> [--keep-existing]");
            std::process::exit(2);
        }
    };
    let clear_existing = match arguments.next().as_deref() {
        None => true,
        Some("--keep-existing") => false,
        Some(other) => {
            eprintln!("unknown argument: {other}");
            std::process::exit(2);
        }
    };
    let store = PostgresLedgerStore::new(url);
    if let Err(error) = store.verify_integrity(&tenant).await {
        eprintln!("integrity verification failed for {}: {error}", tenant.0);
        std::process::exit(1);
    }
    match store
        .rebuild_projections_from_canonical(&tenant, clear_existing)
        .await
    {
        Ok(count) => println!(
            "rebuilt {count} projection(s) for tenant {} from the verified canonical stream",
            tenant.0
        ),
        Err(error) => {
            eprintln!("projection rebuild failed for {}: {error}", tenant.0);
            std::process::exit(1);
        }
    }
}
