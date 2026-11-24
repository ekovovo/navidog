//! Runtime state of open connections, the metadata cache and the completion
//! outline. These need GPUI; the drivers themselves live in `navidog-db`.

pub mod outline;
mod registry;
pub mod scope_cache;
pub mod sessions;

pub use registry::DriverRegistry;

use gpui_kit::App;
use std::sync::Arc;

use navidog_app_state::registry::Source;
use navidog_dameng::DamengDriver;
use navidog_db::Driver;
use navidog_mysql::MySqlDriver;
use navidog_postgres::PostgresDriver;
use navidog_sqlite::SqliteDriver;

pub fn init(cx: &mut App) {
    let mut registry = DriverRegistry::default();
    let builtin: [Arc<dyn Driver>; 5] = [
        Arc::new(MySqlDriver::mysql()),
        Arc::new(MySqlDriver::mariadb()),
        Arc::new(PostgresDriver::default()),
        Arc::new(SqliteDriver::default()),
        Arc::new(DamengDriver::default()),
    ];
    for driver in builtin {
        let id = driver.info().id.clone();
        registry
            .0
            .register(id, Source::Builtin, driver)
            .expect("built-in driver IDs are unique");
    }
    cx.set_global(registry);
    sessions::init(cx);
    outline::init(cx);
}
