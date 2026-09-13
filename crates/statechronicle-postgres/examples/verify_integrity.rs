//! Verify every PostgreSQL tenant before startup or restore promotion.
//!
//! Usage: `STATECHRONICLE_POSTGRES_URL='...' cargo run -p statechronicle-postgres --example verify_integrity`

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
    match PostgresLedgerStore::new_verified(url).await {
        Ok(store) => match store.verify_all_integrity().await {
            Ok(tenants) => {
                println!("verified {} tenant scope(s)", tenants.len());
                for tenant in tenants {
                    println!("verified tenant {}", tenant.0);
                }
            }
            Err(error) => {
                eprintln!("PostgreSQL integrity verification failed: {error}");
                std::process::exit(1);
            }
        },
        Err(error) => {
            eprintln!("PostgreSQL integrity verification failed: {error}");
            std::process::exit(1);
        }
    }
}
