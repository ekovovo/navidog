//! Headless entry point: `navidog-model` and `navidog-db` on their own, with no GPUI dependency.
//!
//! The integration tests in this crate compile and run against those two crates only, so the
//! driver stack cannot quietly grow a window/GPU dependency (A-02). The shell, the navigator and
//! every other pane live in crates that depend on `navidog-app-state` instead.

pub use navidog_dameng as dameng;
pub use navidog_db as db;
pub use navidog_model as model;

#[cfg(test)]
mod tests {
    /// The Dameng driver stack compiles and links without GPUI, and the DriverInfo
    /// flags match the spec (140-dameng.md §2): network and SSH only, no SSL.
    #[test]
    fn dameng_driver_flags_stay_capability_only() {
        use navidog_db::Driver as _;
        let info = crate::dameng::DamengDriver::default().info().clone();
        assert_eq!(info.id, "dameng");
        assert_eq!(info.default_port, Some(5236));
        assert!(info.supports_network && info.supports_ssh);
        assert!(!info.supports_ssl);
        assert!(!info.file_based && !info.multi_host);
        // The dialect capability set: DM folds unquoted names to upper case and has
        // no transactional DDL.
        let features = info.dialect.features();
        assert!(features.folds_unquoted_upper);
        assert!(!features.transactional_ddl);
        assert!(!features.identity_editable);
    }
}
