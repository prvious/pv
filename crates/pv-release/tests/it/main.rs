//! The pv-release crate's integration tests, in one binary so they compile and link once.

mod app_installer;
mod app_publication;
mod app_release_records;
mod archive_validation;
mod manifest_defaults;
mod manifest_generation;
mod publication;
mod recipe_fixtures;
mod recipe_metadata;
mod record_writer;
mod release_records;
mod smoke;
mod workflow_defaults;
