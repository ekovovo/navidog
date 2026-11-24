//! Dameng DM8 driver and SQL dialect. dm-pure is a synchronous protocol client, so all
//! calls run on blocking threads, mirroring `navidog-sqlite`.

pub mod dialect;
pub mod driver;
pub mod metadata;
pub mod words;

pub use dialect::DamengDialect;
pub use driver::DamengDriver;

rust_i18n::i18n!("locales", fallback = "en");
