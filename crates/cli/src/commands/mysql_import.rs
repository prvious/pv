use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::process::{ExitCode, Stdio};
use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};
use config::ProjectConfigFile;
use mysql_dump::{
    RoutineName, discover_source_databases, is_system_database, preflight_dump, routine_reference,
    source_database_header, write_transformed,
};
use platform::clock::{self, Limit};
use resources::TrackSelector;
use sqlx::mysql::{MySqlConnectOptions, MySqlConnection};
use sqlx::{AssertSqlSafe, Connection};
use state::{Database, JobsLock, PortOwner};
use tokio::sync::watch;

use crate::args::MysqlImportArgs;
use crate::environment::Environment;
use crate::error::{CliError, ExecuteError};
use crate::output::Streams;
use crate::prompt;

use super::{project, pv_paths};

const MYSQL_CONNECT_LIMIT: Limit = Limit::new("MySQL import connection", Duration::from_secs(30));

struct Target {
    physical: String,
    allocation: String,
    managed: bool,
}

pub(crate) fn run(
    args: MysqlImportArgs,
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let _jobs_lock = JobsLock::acquire(&paths)?;
    let database = Database::open(&paths)?;
    let project = project::resolve_project(&database, args.project.as_deref(), environment)?;
    let config_file = ProjectConfigFile::read_from_root(&project.path)?;
    let mysql_config = config_file
        .config
        .resources
        .get("mysql")
        .ok_or_else(|| fail("Project config does not declare a MySQL track"))?;
    let requirement = database
        .project_managed_resources(&project.id)?
        .into_iter()
        .find(|resource| resource.resource_name == "mysql")
        .ok_or_else(|| fail("Project MySQL track has not reconciled yet"))?;
    if let Some(selector) = &mysql_config.track
        && let TrackSelector::Track(configured_track) = TrackSelector::parse(selector)?
        && configured_track.as_str() != requirement.track
    {
        return Err(fail(
            "Project MySQL track has changed; reconcile the Project before importing",
        ));
    }
    let track = database.managed_resource_track("mysql", &requirement.track)?;
    let artifact = track
        .current_artifact_path
        .as_ref()
        .ok_or_else(|| fail("Project MySQL track is not installed"))?;
    let client = artifact.join("bin/mysql");
    if !state::fs::path_is_file(&client)? {
        return Err(fail("managed MySQL artifact has no mysql client"));
    }
    let username = track
        .env
        .get("username")
        .ok_or_else(|| fail("MySQL track has no username"))?;
    if username != "pv_root" {
        return Err(fail(
            "MySQL track root account does not match the import definer policy",
        ));
    }
    let password = track
        .env
        .get("password")
        .ok_or_else(|| fail("MySQL track has no password"))?;
    if password.is_empty() || !password.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(fail("MySQL track password is not in PV's generated format"));
    }
    let port = database
        .assigned_ports()?
        .into_iter()
        .find_map(|assignment| match assignment.owner {
            PortOwner::Resource { name, track, port }
                if name == "mysql" && track == requirement.track && port == "mysql" =>
            {
                Some(assignment.port)
            }
            _ => None,
        })
        .ok_or_else(|| fail("MySQL track has no assigned port"))?;
    if !streams.interactive && !args.yes {
        return Err(fail(
            "confirmation requires a terminal; pass --yes to accept the plan",
        ));
    }
    let input = absolute_input_path(&args.path, environment)?;
    if !state::fs::path_is_file(&input)? {
        return Err(fail("dump must be a regular file"));
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()?;
    let (cancel_sender, cancel) = watch::channel(false);
    #[cfg(unix)]
    {
        let (mut interrupt, mut terminate) = {
            let _entered = runtime.enter();
            (
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?,
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?,
            )
        };
        runtime.spawn(async move {
            tokio::select! {
                _ = interrupt.recv() => {},
                _ = terminate.recv() => {},
            }
            cancel_sender.send_replace(true);
        });
    }
    #[cfg(windows)]
    {
        let mut interrupt = {
            let _entered = runtime.enter();
            tokio::signal::windows::ctrl_c()?
        };
        runtime.spawn(async move {
            interrupt.recv().await;
            cancel_sender.send_replace(true);
        });
    }
    state::fs::ensure_user_dir(paths.run())?;
    let temporary = camino_tempfile::tempdir_in(paths.run())?;
    let snapshot = temporary.path().join("source.sql");
    {
        let mut snapshot_file = state::fs::create_new_file(&snapshot)?;
        state::fs::secure_sensitive_file(&snapshot)?;
        let mut source = BufReader::new(open_for_preflight(&input, &cancel)?);
        if source.fill_buf()?.starts_with(b"\xef\xbb\xbf") {
            source.consume(3);
        }
        io::copy(&mut source, &mut snapshot_file)?;
    }
    check_cancelled(&cancel)?;

    let header = source_database_header(open_for_preflight(&snapshot, &cancel)?).map_err(fail)?;
    let mut sources = discover_source_databases(
        open_for_preflight(&snapshot, &cancel)?,
        &mut open_for_preflight(&snapshot, &cancel)?,
    )
    .map_err(fail)?;
    if let Some(header) = &header {
        sources.insert(header.clone());
    }
    check_cancelled(&cancel)?;
    let mappings = parse_mappings(&args.mappings)?;
    let initial = if let Some(header) = &header {
        Some(header.clone())
    } else if sources.is_empty() && mappings.len() == 1 {
        mappings.keys().next().cloned()
    } else {
        None
    };
    if sources.is_empty() && mappings.is_empty() {
        return Err(fail(
            "dump has no database routing or header; pass --map source=allocation",
        ));
    }
    sources.extend(mappings.keys().cloned());
    let declared = database.resource_allocations(&project.id, "mysql")?;
    let mut targets = BTreeMap::new();
    for source in sources.iter().filter(|source| !is_system_database(source)) {
        let allocation = mappings.get(source).map(String::as_str).unwrap_or(source);
        config::validate_allocation_name(allocation)?;
        let managed = mysql_config.allocations.contains_key(allocation);
        let physical = if managed {
            declared
                .iter()
                .find(|record| {
                    record.track == requirement.track && record.allocation_name == allocation
                })
                .ok_or_else(|| {
                    fail(format!(
                        "declared MySQL allocation {allocation} has not reconciled yet"
                    ))
                })?
                .generated_name
                .clone()
        } else {
            resources::generated_allocation_name("mysql", &project.slug, allocation)?
                .generated_name()
                .to_owned()
        };
        if !managed
            && declared.iter().any(|record| {
                record.track == requirement.track
                    && record.generated_name.eq_ignore_ascii_case(&physical)
            })
        {
            return Err(fail(format!(
                "target {physical} is a Project allocation; use --map to select a declared allocation"
            )));
        }
        targets.insert(
            source.clone(),
            Target {
                physical,
                allocation: allocation.to_owned(),
                managed,
            },
        );
    }
    reject_other_project_targets(&database, &project.id, &requirement.track, &targets)?;
    let physical = targets
        .iter()
        .map(|(source, target)| (source.clone(), target.physical.clone()))
        .collect();
    let skip_routines = parse_routine_skips(&args.skip_routines)?;
    let plan = preflight_dump(
        open_for_preflight(&snapshot, &cancel)?,
        &mut open_for_preflight(&snapshot, &cancel)?,
        &physical,
        initial.as_deref(),
        skip_routines,
    )
    .map_err(fail)?;
    check_cancelled(&cancel)?;
    if plan.used_source_databases.is_empty() {
        return Err(fail("dump has no user database statements to import"));
    }
    let used_targets = plan
        .used_source_databases
        .iter()
        .map(|source| {
            targets
                .get(source)
                .map(|target| (source.clone(), target))
                .ok_or_else(|| fail(format!("source database {source} was not mapped")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let transformed = temporary.path().join("transformed.sql");
    let mut transformed_file = state::fs::create_new_file(&transformed)?;
    state::fs::secure_sensitive_file(&transformed)?;
    write_transformed(
        open_for_preflight(&snapshot, &cancel)?,
        &mut transformed_file,
        plan.edits,
    )
    .map_err(fail)?;
    transformed_file.flush()?;
    drop(transformed_file);
    check_cancelled(&cancel)?;

    let options = MySqlConnectOptions::new()
        .host("127.0.0.1")
        .port(port)
        .username(username)
        .password(password);
    let nonempty = runtime.block_on(check_nonempty_targets(&options, &used_targets))?;
    check_cancelled(&cancel)?;
    streams.out.line(&format!(
        "MySQL import plan for {} (MySQL {})",
        project.slug, requirement.track
    ))?;
    streams
        .out
        .line("Source database  Target database  PV state after import")?;
    for (source, target) in &used_targets {
        let state = if target.managed {
            "managed allocation"
        } else {
            "unmanaged database"
        };
        streams
            .out
            .line(&format!("{source}  {}  {state}", target.physical))?;
    }
    if !plan.skipped_system_databases.is_empty() {
        streams.out.line(&format!(
            "System databases skipped: {}",
            plan.skipped_system_databases
                .into_iter()
                .collect::<Vec<_>>()
                .join(", ")
        ))?;
    }
    for routine in &plan.skipped_routines {
        streams.out.line(&format!(
            "Routine skipped: {}.{} (will be missing after import)",
            routine.database, routine.name
        ))?;
    }
    for routine in &plan.truncating_routines {
        streams.out.line(&format!(
            "Routine contains TRUNCATE of mapped tables: {}.{}",
            routine.database, routine.name
        ))?;
    }
    for (source, target) in &used_targets {
        if !target.managed {
            streams.out.line(&format!("{source} imports as {} without a pv.yml allocation. To manage it, add to mysql.allocations:\n  {}: {{}}", target.physical, target.allocation))?;
        }
    }
    if !nonempty.is_empty() {
        streams.out.line(&format!(
            "Existing targets contain objects: {}",
            nonempty.join(", ")
        ))?;
    }
    if !args.yes
        && !prompt::confirm(
            environment,
            &streams.out,
            "Import the planned databases?",
            false,
        )?
    {
        streams.out.line("Import cancelled; no SQL was run.")?;
        return Ok(ExitCode::FAILURE);
    }
    if !nonempty.is_empty() && !args.force {
        if !streams.interactive {
            return Err(fail(
                "existing targets contain objects; pass --force to continue",
            ));
        }
        if !prompt::confirm(
            environment,
            &streams.out,
            "Import into targets that already contain objects?",
            false,
        )? {
            streams.out.line("Import cancelled; no SQL was run.")?;
            return Ok(ExitCode::FAILURE);
        }
    }
    check_cancelled(&cancel)?;
    let option_file = temporary.path().join("client.cnf");
    let default_database = initial
        .as_ref()
        .filter(|name| plan.used_source_databases.contains(*name))
        .and_then(|name| targets.get(name));
    let mut option_contents = format!(
        "[client]\nuser={username}\npassword={password}\nhost=127.0.0.1\nport={port}\nprotocol=tcp\n"
    );
    if let Some(default_database) = default_database {
        option_contents.push_str(&format!("database={}\n", default_database.physical));
    }
    state::fs::write_sensitive_file(&option_file, &option_contents)?;
    let target_names = used_targets
        .iter()
        .map(|(_, target)| target.physical.clone())
        .collect::<Vec<_>>();
    let result = runtime.block_on(execute_import(
        &options,
        &target_names,
        &client,
        &option_file,
        &transformed,
        temporary.path(),
        cancel,
    ));
    match result {
        Ok(()) => {
            streams.out.line("MySQL import completed.")?;
            Ok(ExitCode::SUCCESS)
        }
        Err(error) => {
            streams.out.line(&format!(
                "Import may be partial in: {}",
                target_names.join(", ")
            ))?;
            Err(error)
        }
    }
}

fn check_cancelled(cancel: &watch::Receiver<bool>) -> Result<(), ExecuteError> {
    if *cancel.borrow() {
        Err(fail("import cancelled; no SQL was run"))
    } else {
        Ok(())
    }
}

fn absolute_input_path(
    path: &std::path::Path,
    environment: &impl Environment,
) -> Result<Utf8PathBuf, ExecuteError> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        environment.current_dir()?.join(path)
    };
    Utf8PathBuf::from_path_buf(path).map_err(|path| CliError::NonUtf8Path { path }.into())
}

fn parse_mappings(values: &[String]) -> Result<BTreeMap<String, String>, ExecuteError> {
    let mut mappings = BTreeMap::new();
    for value in values {
        let (source, allocation) = value
            .rsplit_once('=')
            .ok_or_else(|| fail(format!("invalid --map {value}; expected source=allocation")))?;
        if source.is_empty()
            || allocation.is_empty()
            || mappings
                .insert(source.to_owned(), allocation.to_owned())
                .is_some()
        {
            return Err(fail(format!("duplicate or invalid --map {value}")));
        }
        config::validate_allocation_name(allocation)?;
    }
    Ok(mappings)
}

fn parse_routine_skips(values: &[String]) -> Result<Vec<RoutineName>, ExecuteError> {
    values
        .iter()
        .map(|value| {
            let source = format!("DROP PROCEDURE {value};");
            let reference = routine_reference(&source, b";")
                .map_err(fail)?
                .ok_or_else(|| fail(format!("invalid --skip-routine {value}")))?;
            let database = reference.database.ok_or_else(|| {
                fail(format!(
                    "invalid --skip-routine {value}; expected database.routine"
                ))
            })?;
            Ok(RoutineName {
                database,
                name: reference.name,
            })
        })
        .collect()
}

fn reject_other_project_targets(
    database: &Database,
    project_id: &str,
    track: &str,
    targets: &BTreeMap<String, Target>,
) -> Result<(), ExecuteError> {
    let names = targets
        .values()
        .map(|target| target.physical.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    for other in database.projects()? {
        if other.id == project_id {
            continue;
        }
        for allocation in database.resource_allocations(&other.id, "mysql")? {
            if allocation.track == track
                && names.contains(&allocation.generated_name.to_ascii_lowercase())
            {
                return Err(fail(format!(
                    "target {} belongs to another Project",
                    allocation.generated_name
                )));
            }
        }
    }
    Ok(())
}

// The private snapshot makes each pass stable. Check cancellation as it streams,
// including inside a statement too large for the in-memory parser.
fn open_for_preflight<'a>(
    path: &Utf8Path,
    cancel: &'a watch::Receiver<bool>,
) -> Result<impl Read + Seek + use<'a>, ExecuteError> {
    Ok(CancellableReader {
        reader: open_required(path)?,
        cancel,
    })
}

struct CancellableReader<'a, R> {
    reader: R,
    cancel: &'a watch::Receiver<bool>,
}

impl<R: Read> Read for CancellableReader<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if *self.cancel.borrow() {
            return Err(io::Error::other("import cancelled; no SQL was run"));
        }
        self.reader.read(buffer)
    }
}

impl<R: Seek> Seek for CancellableReader<'_, R> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.reader.seek(position)
    }
}

#[expect(
    clippy::disallowed_types,
    reason = "MySQL client stdin needs a file handle"
)]
fn open_required(path: &Utf8Path) -> Result<std::fs::File, ExecuteError> {
    state::fs::open_read_file(path)?.ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, format!("file missing: {path}")).into()
    })
}

async fn connect(options: &MySqlConnectOptions) -> Result<MySqlConnection, ExecuteError> {
    clock::timeout(&MYSQL_CONNECT_LIMIT, MySqlConnection::connect_with(options))
        .await
        .map_err(|error| fail(format!("connecting to the MySQL track: {error}")))?
        .map_err(|error| fail(format!("connecting to the MySQL track: {error}")))
}

async fn check_nonempty_targets(
    options: &MySqlConnectOptions,
    targets: &[(String, &Target)],
) -> Result<Vec<String>, ExecuteError> {
    let mut connection = connect(options).await?;
    let mut nonempty = Vec::new();
    for (_, target) in targets {
        let count: i64 = sqlx::query_scalar(
            "SELECT CAST((SELECT COUNT(*) FROM information_schema.tables WHERE table_schema=?) + (SELECT COUNT(*) FROM information_schema.routines WHERE routine_schema=?) + (SELECT COUNT(*) FROM information_schema.events WHERE event_schema=?) AS SIGNED)",
        )
        .bind(&target.physical)
        .bind(&target.physical)
        .bind(&target.physical)
        .fetch_one(&mut connection)
        .await
        .map_err(|error| {
            fail(format!(
                "checking whether target {} contains objects: {error}",
                target.physical
            ))
        })?;
        if count > 0 {
            nonempty.push(target.physical.clone());
        }
    }
    Ok(nonempty)
}

#[expect(
    clippy::disallowed_types,
    reason = "MySQL import owns the managed mysql client child process"
)]
async fn execute_import(
    options: &MySqlConnectOptions,
    targets: &[String],
    client: &Utf8Path,
    option_file: &Utf8Path,
    transformed: &Utf8Path,
    temporary: &Utf8Path,
    mut cancel: watch::Receiver<bool>,
) -> Result<(), ExecuteError> {
    if *cancel.borrow() {
        return Err(fail("import cancelled"));
    }
    let mut connection = connect(options).await?;
    for target in targets {
        if *cancel.borrow() {
            return Err(fail("import cancelled"));
        }
        if target.is_empty()
            || target.len() > 63
            || !target
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err(fail("invalid physical MySQL target database name"));
        }
        // MySQL cannot bind identifiers; preflight and this boundary both
        // constrain target names before constructing SQL.
        sqlx::query(AssertSqlSafe(format!(
            "CREATE DATABASE IF NOT EXISTS `{target}`"
        )))
        .execute(&mut connection)
        .await
        .map_err(|error| fail(format!("creating target database {target}: {error}")))?;
    }
    if *cancel.borrow() {
        return Err(fail("import cancelled"));
    }
    drop(connection);
    let stderr_path = temporary.join("client.stderr");
    let stderr = state::fs::create_new_file(&stderr_path)?;
    state::fs::secure_sensitive_file(&stderr_path)?;
    let mut child = tokio::process::Command::new(client.as_std_path())
        .arg(format!("--defaults-file={option_file}"))
        .arg("--no-login-paths")
        .arg("--binary-mode")
        .arg("--skip-reconnect")
        .arg("--local-infile=0")
        .stdin(Stdio::from(open_required(transformed)?))
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr))
        .kill_on_drop(true)
        .spawn()?;
    let status = tokio::select! {
        biased;
        status = child.wait() => status?,
        changed = cancel.changed() => {
            changed.map_err(|error| fail(format!("cancellation handler failed: {error}")))?;
            child.kill().await?;
            return Err(fail("import cancelled"));
        }
    };
    if !status.success() {
        let mut stderr = open_required(&stderr_path)?;
        let mut prefix = Vec::new();
        Read::by_ref(&mut stderr)
            .take(512)
            .read_to_end(&mut prefix)?;
        let detail = mysql_error_summary(&prefix);
        return Err(fail(format!("mysql client exited with {status}{detail}")));
    }
    Ok(())
}

fn mysql_error_summary(stderr: &[u8]) -> String {
    let message = String::from_utf8_lossy(stderr);
    let Some(first_line) = message.lines().find(|line| line.starts_with("ERROR ")) else {
        return String::new();
    };
    let Some((prefix, _)) = first_line.split_once(':') else {
        return String::new();
    };
    let words = prefix.split_whitespace().collect::<Vec<_>>();
    let (code, state) = match words.as_slice() {
        ["ERROR", code, state] => (*code, *state),
        ["ERROR", code, state, "at", "line", line] if ascii_number(line) => (*code, *state),
        _ => return String::new(),
    };
    if ascii_number(code)
        && state
            .strip_prefix('(')
            .and_then(|value| value.strip_suffix(')'))
            .is_some_and(|value| {
                value.len() == 5 && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
            })
    {
        return format!(": {prefix}");
    }
    String::new()
}

fn ascii_number(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn fail(error: impl std::fmt::Display) -> ExecuteError {
    CliError::MysqlImport {
        message: error.to_string(),
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::mysql_error_summary;

    #[test]
    fn mysql_client_error_reports_code_without_dump_values() {
        let stderr = b"mysql: [Warning] option file warning\nERROR 1062 (23000) at line 42: Duplicate entry 'private row value' for key 'users.PRIMARY'\n";
        assert_eq!(
            mysql_error_summary(stderr),
            ": ERROR 1062 (23000) at line 42"
        );
        assert_eq!(
            mysql_error_summary(b"ERROR 2003 (HY000): Connection refused"),
            ": ERROR 2003 (HY000)"
        );
    }
}
