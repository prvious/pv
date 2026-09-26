//! Bounded-memory framing for MySQL client SQL dumps.

mod header;
mod key_toggle;
mod policy;
mod reader;
mod reference;
mod rewrite;
mod routine;
mod routing;
mod session;
mod stored_routine;
mod table;

pub use header::{HeaderError, source_database_header};
pub use key_toggle::{KeyToggleError, dump_key_toggle};
pub use policy::{StoredRoutinePolicyError, validate_stored_routine_effects};
pub use reader::{Frame, ReaderError, scan};
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
