use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Read, Seek, SeekFrom};
use std::ops::Range;

use squonk::dialect::{BuiltinDialect, tokenize_with_builtin};
use squonk::tokenizer::Token;
use thiserror::Error;

use crate::insert::large_insert_references;
use crate::key_toggle::dump_key_toggle;
use crate::policy::{
    validate_statement_effects, validate_stored_object_effects,
    validate_stored_routine_effects_for_targets,
};
use crate::reader::{Frame, ReaderError, scan_with};
use crate::reference::{
    ReferenceError, StatementReferences, statement_references, verified_identifier,
};
use crate::rewrite::{Edit, Patch};
use crate::routine::{
    RoutineAction, RoutineName, RoutineSkipError, RoutineSkips, routine_reference,
};
use crate::routing::{RoutingAction, routing_reference};
use crate::session::{SessionSetup, session_setup};
use crate::stored_routine::analyze_stored_routine;
use crate::table::table_database_references;
use crate::view::view_statement_references;

const MAX_STATEMENT_BYTES: u64 = 8 * 1024 * 1024;
const DEFINER_REPLACEMENT: &[u8] = b"DEFINER='pv_root'@'127.0.0.1'";

/// Complete preflight output; callers transform an immutable snapshot with
/// these edits before opening the MySQL client.
#[derive(Debug)]
pub struct ImportPlan {
    pub edits: Vec<Edit>,
    pub used_source_databases: BTreeSet<String>,
    pub skipped_routines: Vec<RoutineName>,
    pub truncating_routines: Vec<RoutineName>,
    pub skipped_system_databases: BTreeSet<String>,
}

#[derive(Debug, Error)]
pub enum PreflightError {
    #[error(transparent)]
    Reader(#[from] ReaderError),
    #[error("could not inspect dump: {0}")]
    Io(#[from] io::Error),
    #[error("SQL statement at byte {offset} is not UTF-8")]
    InvalidUtf8 { offset: u64 },
    #[error("could not tokenize SQL at byte {offset}: {message}")]
    Tokenize { offset: u64, message: String },
    #[error("statement at byte {offset} has no mapped source database")]
    MissingDatabase { offset: u64 },
    #[error("source database {database} has no target mapping")]
    UnmappedDatabase { database: String },
    #[error("invalid physical MySQL target database name: {database}")]
    InvalidTarget { database: String },
    #[error("multiple source databases map to physical target {database}")]
    TargetCollision { database: String },
    #[error("user SQL references system database {database}")]
    SystemDatabaseReference { database: String },
    #[error(
        "statement at byte {offset} references user database {database} inside a skipped system section"
    )]
    MixedSystemSection { offset: u64, database: String },
    #[error("source identifier at byte {offset} has no verified span")]
    InvalidSpan { offset: u64 },
    #[error("unsupported SQL at byte {offset}: {message}")]
    Unsupported { offset: u64, message: String },
    #[error(transparent)]
    RoutineSkip(#[from] RoutineSkipError),
}

/// Callers must provide two readers over the same immutable SQL snapshot. The
/// first is scanned sequentially; the second seeks to bounded statement ranges.
pub fn preflight_dump<R: Read, S: Read + Seek>(
    scan_source: R,
    inspect_source: &mut S,
    targets: &BTreeMap<String, String>,
    header_database: Option<&str>,
    requested_skips: impl IntoIterator<Item = RoutineName>,
) -> Result<ImportPlan, PreflightError> {
    let mut target_names = BTreeSet::new();
    for target in targets.values() {
        let valid = target.len() <= 63
            && target
                .as_bytes()
                .first()
                .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
            && target
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            && !is_system_database(target);
        if !valid {
            return Err(PreflightError::InvalidTarget {
                database: target.clone(),
            });
        }
        if !target_names.insert(target.to_ascii_lowercase()) {
            return Err(PreflightError::TargetCollision {
                database: target.clone(),
            });
        }
    }
    let mut active_database = header_database.map(str::to_owned);
    let mapped_databases = targets.keys().cloned().collect::<BTreeSet<_>>();
    let mut skips = RoutineSkips::new(requested_skips)?;
    let mut plan = ImportPlan {
        edits: Vec::new(),
        used_source_databases: BTreeSet::new(),
        skipped_routines: Vec::new(),
        truncating_routines: Vec::new(),
        skipped_system_databases: BTreeSet::new(),
    };
    scan_with(scan_source, |frame| {
        let Frame::Sql { range, delimiter } = frame else {
            return Ok(());
        };
        let length = range.end - range.start;
        if length > MAX_STATEMENT_BYTES {
            let (prefix, references) =
                large_insert_references(inspect_source, range.clone(), &delimiter)
                    .map_err(|error| unsupported(range.start, error))?;
            if active_database.as_deref().is_some_and(is_system_database) {
                if let Some(reference) = references
                    .databases
                    .iter()
                    .find(|reference| !is_system_database(&reference.name))
                {
                    return Err(PreflightError::MixedSystemSection {
                        offset: range.start,
                        database: reference.name.clone(),
                    });
                }
                plan.edits.push(Edit::Skip(range));
                return Ok(());
            }
            let patches = reference_patches(
                &prefix,
                range.start,
                references,
                active_database.as_deref(),
                targets,
                &mut plan.used_source_databases,
                Some,
            )?;
            plan.edits.extend(patches);
            return Ok(());
        }
        inspect_source.seek(SeekFrom::Start(range.start))?;
        let mut source = Vec::with_capacity(length as usize);
        inspect_source.take(length).read_to_end(&mut source)?;
        if source.len() != length as usize {
            return Err(PreflightError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "dump changed during preflight",
            )));
        }
        let source = std::str::from_utf8(&source).map_err(|_| PreflightError::InvalidUtf8 {
            offset: range.start,
        })?;
        let tokens = tokenize_with_builtin(source, BuiltinDialect::MySql).map_err(|error| {
            PreflightError::Tokenize {
                offset: range.start,
                message: error.to_string(),
            }
        })?;
        let text =
            |token: &Token| source.get(token.span.start() as usize..token.span.end() as usize);
        let word = |index: usize, expected: &str| {
            tokens
                .get(index)
                .and_then(text)
                .is_some_and(|word| word.eq_ignore_ascii_case(expected))
        };
        let is_routing = word(0, "USE")
            || (["CREATE", "ALTER", "DROP"].iter().any(|verb| word(0, verb))
                && (word(1, "DATABASE") || word(1, "SCHEMA")));
        if is_routing {
            let routing = routing_reference(source)
                .map_err(|error| unsupported(range.start, error))?
                .ok_or_else(|| unsupported(range.start, "unrecognized database routing"))?;
            if routing.action == RoutingAction::Use {
                active_database = Some(routing.name.clone());
            }
            if is_system_database(&routing.name) {
                plan.skipped_system_databases.insert(routing.name);
                plan.edits.push(Edit::Skip(range));
            } else if routing.action == RoutingAction::Drop {
                // Never delete the allocation container during an import.
                plan.edits.push(Edit::Skip(range));
            } else {
                let target = target_for(&routing.name, targets)?;
                plan.used_source_databases.insert(routing.name.clone());
                if routing.action == RoutingAction::Create {
                    // PV creates the mapped databases with provider defaults before replay.
                    plan.edits.push(Edit::Skip(range));
                } else {
                    plan.edits.push(Edit::Patch(name_patch(
                        source,
                        range.start,
                        routing.span,
                        target,
                    )?));
                }
            }
            return Ok(());
        }
        if active_database.as_deref().is_some_and(is_system_database) {
            for pair in tokens.windows(2) {
                let identifier_span = pair[0].span.start() as usize..pair[0].span.end() as usize;
                if text(&pair[1]) == Some(".")
                    && let Some(database) = targets.keys().find(|database| {
                        verified_identifier(source, identifier_span.clone(), database)
                    })
                {
                    return Err(PreflightError::MixedSystemSection {
                        offset: range.start,
                        database: database.clone(),
                    });
                }
            }
            plan.edits.push(Edit::Skip(range));
            return Ok(());
        }
        let routine = routine_reference(source, &delimiter)
            .map_err(|error| unsupported(range.start, error))?;
        if let Some(routine) = &routine
            && skips.observe(routine, active_database.as_deref())?
        {
            plan.edits.push(Edit::Skip(range));
            return Ok(());
        }
        if word(0, "SET") {
            match session_setup(source).map_err(|error| unsupported(range.start, error))? {
                Some(SessionSetup::Skip) => plan.edits.push(Edit::Skip(range)),
                Some(SessionSetup::Keep) => {}
                None => return Err(unsupported(range.start, "unsupported SET statement")),
            }
            return Ok(());
        }
        if dump_key_toggle(source).map_err(|error| unsupported(range.start, error))? {
            plan.edits.push(Edit::Skip(range));
            return Ok(());
        }
        let active = active_database.as_deref();
        let mut patches = if let Some(routine) = routine
            && routine.action == RoutineAction::Create
        {
            let analysis = analyze_stored_routine(source, &delimiter)
                .map_err(|error| unsupported(range.start, error))?;
            let routine_database =
                routine
                    .database
                    .as_deref()
                    .or(active)
                    .ok_or(PreflightError::MissingDatabase {
                        offset: range.start,
                    })?;
            let truncates = validate_stored_routine_effects_for_targets(
                &analysis.sql,
                Some(routine_database),
                &mapped_databases,
            )
            .map_err(|error| unsupported(range.start, error))?;
            if truncates {
                plan.truncating_routines.push(RoutineName {
                    database: routine_database.to_owned(),
                    name: routine.name,
                });
            }
            let references = statement_references(&analysis.sql)
                .map_err(|error| unsupported(range.start, error))?;
            reference_patches(
                source,
                range.start,
                references,
                active,
                targets,
                &mut plan.used_source_databases,
                |span| analysis.source_span(span),
            )?
        } else if word(0, "CREATE")
            && (word(1, "TABLE") || (word(1, "TEMPORARY") && word(2, "TABLE")))
        {
            let databases = table_database_references(source)
                .map_err(|error| unsupported(range.start, error))?;
            reference_patches(
                source,
                range.start,
                StatementReferences {
                    databases,
                    definers: Vec::new(),
                },
                active,
                targets,
                &mut plan.used_source_databases,
                Some,
            )?
        } else if word(0, "CREATE") && (word(1, "VIEW") || word(1, "ALGORITHM")) {
            let references = match statement_references(source) {
                Ok(references) => {
                    validate_statement_effects(source)
                        .map_err(|error| unsupported(range.start, error))?;
                    references
                }
                Err(ReferenceError::UnsupportedSyntax { .. }) => view_statement_references(source)
                    .map_err(|error| unsupported(range.start, error))?,
                Err(error) => return Err(unsupported(range.start, error)),
            };
            reference_patches(
                source,
                range.start,
                references,
                active,
                targets,
                &mut plan.used_source_databases,
                Some,
            )?
        } else {
            let normalized = if delimiter == b";" {
                source.to_owned()
            } else {
                let ending = std::str::from_utf8(&delimiter)
                    .map_err(|_| unsupported(range.start, "invalid delimiter"))?;
                let prefix = source
                    .strip_suffix(ending)
                    .ok_or_else(|| unsupported(range.start, "missing statement delimiter"))?;
                format!("{prefix};{}", " ".repeat(ending.len() - 1))
            };
            if validate_stored_object_effects(&normalized).is_err() {
                validate_statement_effects(&normalized)
                    .map_err(|error| unsupported(range.start, error))?;
            }
            let references = statement_references(&normalized)
                .map_err(|error| unsupported(range.start, error))?;
            reference_patches(
                source,
                range.start,
                references,
                active,
                targets,
                &mut plan.used_source_databases,
                Some,
            )?
        };
        patches.sort_by_key(|edit| match edit {
            Edit::Patch(patch) => patch.range.start,
            Edit::Skip(range) => range.start,
        });
        plan.edits.extend(patches);
        Ok(())
    })?;
    plan.skipped_routines = skips.finish()?;
    Ok(plan)
}

fn require_source(
    active: Option<&str>,
    has_qualifier: bool,
    offset: u64,
    targets: &BTreeMap<String, String>,
    used_databases: &mut BTreeSet<String>,
) -> Result<(), PreflightError> {
    if let Some(active) = active {
        target_for(active, targets)?;
        used_databases.insert(active.to_owned());
    } else if !has_qualifier {
        return Err(PreflightError::MissingDatabase { offset });
    }
    Ok(())
}

fn reference_patches(
    source: &str,
    offset: u64,
    references: StatementReferences,
    active: Option<&str>,
    targets: &BTreeMap<String, String>,
    used_databases: &mut BTreeSet<String>,
    source_span: impl Fn(Range<usize>) -> Option<Range<usize>>,
) -> Result<Vec<Edit>, PreflightError> {
    require_source(
        active,
        !references.databases.is_empty(),
        offset,
        targets,
        used_databases,
    )?;
    let mut edits = Vec::new();
    for reference in references.databases {
        if is_system_database(&reference.name) {
            return Err(PreflightError::SystemDatabaseReference {
                database: reference.name,
            });
        }
        let target = target_for(&reference.name, targets)?;
        used_databases.insert(reference.name.clone());
        let span = source_span(reference.span).ok_or(PreflightError::InvalidSpan { offset })?;
        if !verified_identifier(source, span.clone(), &reference.name) {
            return Err(PreflightError::InvalidSpan { offset });
        }
        edits.push(Edit::Patch(name_patch(source, offset, span, target)?));
    }
    for definer in references.definers {
        let span = source_span(definer).ok_or(PreflightError::InvalidSpan { offset })?;
        let expected = source
            .get(span.clone())
            .ok_or(PreflightError::InvalidSpan { offset })?;
        if !expected.to_ascii_uppercase().starts_with("DEFINER") {
            return Err(PreflightError::InvalidSpan { offset });
        }
        edits.push(Edit::Patch(Patch {
            range: offset + span.start as u64..offset + span.end as u64,
            expected: expected.as_bytes().to_vec(),
            replacement: DEFINER_REPLACEMENT.to_vec(),
        }));
    }
    Ok(edits)
}

fn name_patch(
    source: &str,
    offset: u64,
    span: Range<usize>,
    target: &str,
) -> Result<Patch, PreflightError> {
    let expected = source
        .get(span.clone())
        .ok_or(PreflightError::InvalidSpan { offset })?;
    let replacement = if expected.starts_with('`') {
        format!("`{target}`")
    } else {
        target.to_owned()
    };
    Ok(Patch {
        range: offset + span.start as u64..offset + span.end as u64,
        expected: expected.as_bytes().to_vec(),
        replacement: replacement.into_bytes(),
    })
}

fn target_for<'a>(
    name: &str,
    targets: &'a BTreeMap<String, String>,
) -> Result<&'a str, PreflightError> {
    targets
        .get(name)
        .map(String::as_str)
        .ok_or_else(|| PreflightError::UnmappedDatabase {
            database: name.to_owned(),
        })
}

pub fn is_system_database(name: &str) -> bool {
    ["mysql", "sys", "performance_schema", "information_schema"]
        .iter()
        .any(|system| name.eq_ignore_ascii_case(system))
}

fn unsupported(offset: u64, error: impl std::fmt::Display) -> PreflightError {
    PreflightError::Unsupported {
        offset,
        message: error.to_string(),
    }
}
