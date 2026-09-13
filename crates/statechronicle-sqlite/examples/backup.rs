//! Creates an integrity-checked SQLite snapshot.

use std::env;
use std::error::Error;
use std::io::{Error as IoError, ErrorKind};
use std::path::Path;

use statechronicle_sqlite::SqliteLedgerStore;

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args_os().skip(1);
    let source = args
        .next()
        .ok_or_else(|| IoError::new(ErrorKind::InvalidInput, "missing source database path"))?;
    let destination = args.next().ok_or_else(|| {
        IoError::new(ErrorKind::InvalidInput, "missing destination database path")
    })?;
    if args.next().is_some() {
        return Err(IoError::new(
            ErrorKind::InvalidInput,
            "usage: backup <source-db> <destination-db>",
        )
        .into());
    }
    let store = SqliteLedgerStore::open(Path::new(&source))?;
    store.backup_to(Path::new(&destination))?;
    let reports = SqliteLedgerStore::open(Path::new(&destination))?.verify_all_integrity()?;
    println!("backup verified: {} tenant(s)", reports.len());
    Ok(())
}
