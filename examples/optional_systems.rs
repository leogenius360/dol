//! Facade feature adoption smoke for engine cache and migration policy APIs.

#[cfg(all(feature = "engine", feature = "migrate"))]
fn main() -> dol::core::diagnostic::Result<()> {
    let cache = dol::engine::CachePolicy::default();
    cache.validate()?;
    let migration = dol::migrate::MigrationPolicy::default();
    println!("cache={cache:?}, migration={migration:?}");
    Ok(())
}

#[cfg(not(all(feature = "engine", feature = "migrate")))]
fn main() {
    eprintln!("run with `--features engine,migrate`");
}
