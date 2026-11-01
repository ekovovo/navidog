//! Table drafts and schema changes: parse a `CREATE TABLE` definition into a lossless draft,
//! edit columns, constraints, indexes and options, and plan the guarded SQL that applies the
//! difference. No UI: the designer owns the draft widgets, this crate owns the model and the
//! dialect-specific SQL. `navidog-db` keeps only the driver traits, dialects, metadata and SQL
//! scanning.

pub mod object_design;
pub mod table_design;

#[cfg(test)]
mod security_plan;
