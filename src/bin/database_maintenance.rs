use std::error::Error;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use sunswitcher::persistence::Database;

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args_os().skip(1);
    let path = PathBuf::from(
        args.next()
            .ok_or("usage: database_maintenance <existing-db> [--vacuum]")?,
    );
    let vacuum = match args.next() {
        None => false,
        Some(flag) if flag == "--vacuum" => true,
        Some(_) => return Err("expected --vacuum".into()),
    };
    if args.next().is_some() {
        return Err("unexpected arguments".into());
    }
    let mut database = Database::open_existing(&path)?;
    let now_ms = i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis())?;
    let report = database.maintain(now_ms)?;
    println!(
        "maintenance_at_ms={now_ms} report={report:?} schema={}",
        database.schema_version()?
    );
    if vacuum {
        database.vacuum_exclusive()?;
        println!("vacuum=ok");
    }
    drop(database);
    println!("database_bytes={}", std::fs::metadata(&path)?.len());
    let reopened = Database::open_existing(&path)?;
    reopened.settings()?;
    reopened.load_application_overrides()?;
    reopened.load_user_lexicon()?;
    reopened.load_ignored_words()?;
    println!("production_reopen=ok schema={}", reopened.schema_version()?);
    Ok(())
}
