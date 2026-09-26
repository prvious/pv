//! Bounded-memory framing for MySQL client SQL dumps.

mod discover;
mod header;
mod insert;
mod key_toggle;
mod policy;
mod preflight;
mod reader;
mod reference;
mod rewrite;
mod routine;
mod routing;
mod session;
mod span;
mod stored_routine;
mod table;
mod view;

pub use discover::{DiscoverError, discover_source_databases};
pub use header::{HeaderError, source_database_header};
pub use key_toggle::{KeyToggleError, dump_key_toggle};
pub use policy::{
    StatementPolicyError, StoredRoutinePolicyError, validate_statement_effects,
    validate_stored_object_effects, validate_stored_routine_effects,
    validate_stored_routine_effects_for_targets,
};
pub use preflight::{ImportPlan, PreflightError, preflight_dump};
pub use reader::{Frame, ReaderError, scan, scan_with};
pub use reference::{
    DatabaseReference, ReferenceError, StatementReferences, database_references,
    statement_references,
};
pub use rewrite::{Edit, Patch, RewriteError, write_patched, write_transformed};
pub use routine::{
    RoutineAction, RoutineError, RoutineKind, RoutineName, RoutineReference, RoutineSkipError,
    RoutineSkips, routine_reference,
};
pub use routing::{RoutingAction, RoutingError, RoutingReference, routing_reference};
pub use session::{SessionError, SessionSetup, session_setup};
pub use stored_routine::{
    StoredRoutineAnalysis, StoredRoutineError, analyze_stored_routine,
    normalize_stored_routine_for_analysis,
};
pub use table::{TableError, table_database_references};
pub use view::{ViewError, view_statement_references};
