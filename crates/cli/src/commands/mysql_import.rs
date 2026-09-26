use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::process::{ExitCode, Stdio};

use camino::{Utf8Path, Utf8PathBuf};
use config::ProjectConfigFile;
use mysql_dump::{
    RoutineName, discover_source_databases, preflight_dump, routine_reference,
    source_database_header, write_transformed,
};
use sqlx::AssertSqlSafe;
use sqlx::mysql::{MySqlConnectOptions, MySqlPool};
use state::{Database, JobsLock, PortOwner};
use tokio::sync::watch;

use crate::args::MysqlImportArgs;
use crate::environment::Environment;
use crate::error::{CliError, ExecuteError};
use crate::output::{Output, OutputMode};

use super::{project, pv_paths};

struct Target {
    physical: String,
    allocation: String,
    managed: bool,
}

pub(crate) fn run(
    args: MysqlImportArgs,
    environment: &impl Environment,
    stdout: &mut impl Write,
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
    let input = absolute_input_path(&args.path, environment)?;
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
    state::fs::copy_file_atomically(&input, &snapshot)?;
    state::fs::secure_sensitive_file(&snapshot)?;
    check_cancelled(&cancel)?;

    let header = source_database_header(state::fs::open_read_file(&snapshot)?).map_err(fail)?;
    let mut sources = discover_source_databases(
        state::fs::open_read_file(&snapshot)?,
        &mut state::fs::open_read_file(&snapshot)?,
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
    if sources.is_empty() {
        let Some(source) = initial.as_ref() else {
            return Err(fail(
                "dump has no database routing or header; pass --map source=allocation",
            ));
        };
        sources.insert(source.clone());
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
        state::fs::open_read_file(&snapshot)?,
        &mut state::fs::open_read_file(&snapshot)?,
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
        state::fs::open_read_file(&snapshot)?,
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
    let mut output = Output::new(stdout, OutputMode::plain());
    output.line(&format!(
        "MySQL import plan for {} (MySQL {})",
        project.slug, requirement.track
    ))?;
    output.line("Source database  Target database  PV state after import")?;
    for (source, target) in &used_targets {
        let state = if target.managed {
            "managed allocation"
        } else {
            "unmanaged database"
        };
        output.line(&format!("{source}  {}  {state}", target.physical))?;
    }
    if !plan.skipped_system_databases.is_empty() {
        output.line(&format!(
            "System databases skipped: {}",
            plan.skipped_system_databases
                .into_iter()
                .collect::<Vec<_>>()
                .join(", ")
        ))?;
    }
    for routine in &plan.skipped_routines {
        output.line(&format!(
            "Routine skipped: {}.{} (will be missing after import)",
            routine.database, routine.name
        ))?;
    }
    for routine in &plan.truncating_routines {
        output.line(&format!(
            "Routine contains TRUNCATE of mapped tables: {}.{}",
            routine.database, routine.name
        ))?;
    }
    for (source, target) in &used_targets {
        if !target.managed {
            output.line(&format!("{source} imports as {} without a pv.yml allocation. To manage it, add to mysql.allocations:\n  {}: {{}}", target.physical, target.allocation))?;
        }
    }
    if !nonempty.is_empty() {
        output.line(&format!(
            "Existing targets contain objects: {}",
            nonempty.join(", ")
        ))?;
    }
    if !args.yes
        && !confirm(
            environment,
            &mut output,
            "Type `yes` to import the planned databases.",
        )?
    {
        output.line("Import cancelled; no SQL was run.")?;
        return Ok(ExitCode::FAILURE);
    }
    if !nonempty.is_empty()
        && !args.force
        && !confirm(
            environment,
            &mut output,
            "Existing targets contain objects. Type `yes` again to continue, or rerun with --force.",
        )?
    {
        output.line("Import cancelled; no SQL was run.")?;
        return Ok(ExitCode::FAILURE);
    }
    check_cancelled(&cancel)?;
    let option_file = temporary.path().join("client.cnf");
    let default_database = initial
        .as_ref()
        .filter(|name| plan.used_source_databases.contains(*name))
        .and_then(|name| targets.get(name))
        .or_else(|| (used_targets.len() == 1).then(|| used_targets[0].1));
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
            output.line("MySQL import completed.")?;
            for (source, target) in &used_targets {
                if !target.managed {
                    output.line(&format!("Unmanaged database: {source} → {}. Add mysql.allocations.{} to pv.yml to adopt it.", target.physical, target.allocation))?;
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        Err(error) => {
            output.line(&format!(
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

fn is_system_database(name: &str) -> bool {
    ["mysql", "sys", "performance_schema", "information_schema"]
        .iter()
        .any(|system| name.eq_ignore_ascii_case(system))
}

fn confirm(
    environment: &impl Environment,
    output: &mut Output<'_, impl Write>,
    prompt: &str,
) -> Result<bool, ExecuteError> {
    if !environment.stdin_is_terminal() {
        return Err(fail(
            "confirmation requires a terminal; pass --yes for the plan and --force for nonempty targets",
        ));
    }
    output.line(prompt)?;
    Ok(environment.read_line()?.trim() == "yes")
}

async fn check_nonempty_targets(
    options: &MySqlConnectOptions,
    targets: &[(String, &Target)],
) -> Result<Vec<String>, ExecuteError> {
    let pool = MySqlPool::connect_with(options.clone())
        .await
        .map_err(|error| fail(format!("connecting to the MySQL track: {error}")))?;
    let mut nonempty = Vec::new();
    for (_, target) in targets {
        let count: i64 = sqlx::query_scalar(
            "SELECT CAST((SELECT COUNT(*) FROM information_schema.tables WHERE table_schema=?) + (SELECT COUNT(*) FROM information_schema.routines WHERE routine_schema=?) + (SELECT COUNT(*) FROM information_schema.events WHERE event_schema=?) AS SIGNED)",
        )
        .bind(&target.physical)
        .bind(&target.physical)
        .bind(&target.physical)
        .fetch_one(&pool)
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
    pool.close().await;
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
    let pool = MySqlPool::connect_with(options.clone())
        .await
        .map_err(|error| fail(format!("connecting to the MySQL track: {error}")))?;
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
        .execute(&pool)
        .await
        .map_err(|error| fail(format!("creating target database {target}: {error}")))?;
    }
    pool.close().await;
    if *cancel.borrow() {
        return Err(fail("import cancelled"));
    }
    let stderr_path = temporary.join("client.stderr");
    let stderr = state::fs::create_new_file(&stderr_path)?;
    state::fs::secure_sensitive_file(&stderr_path)?;
    let mut child = tokio::process::Command::new(client.as_std_path())
        .arg(format!("--defaults-file={option_file}"))
        .arg("--binary-mode")
        .stdin(Stdio::from(state::fs::open_read_file(transformed)?))
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr))
        .kill_on_drop(true)
        .spawn()?;
    let status = tokio::select! {
        biased;
        status = child.wait() => status?,
        changed = cancel.changed() => {
            changed.map_err(|error| fail(format!("cancellation handler failed: {error}")))?;
            if let Some(status) = child.try_wait()? {
                status
            } else {
            child.start_kill()?;
                let status = child.wait().await?;
                if status.success() {
                    status
                } else {
                    return Err(fail("import cancelled"));
                }
            }
        }
    };
    if !status.success() {
        let mut stderr = state::fs::open_read_file(&stderr_path)?;
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
    if words.len() == 6
        && words[0] == "ERROR"
        && words[1].bytes().all(|byte| byte.is_ascii_digit())
        && words[2].starts_with('(')
        && words[2].ends_with(')')
        && words[2][1..words[2].len() - 1]
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric())
        && words[3] == "at"
        && words[4] == "line"
        && words[5].bytes().all(|byte| byte.is_ascii_digit())
    {
        format!(": {prefix}")
    } else {
        String::new()
    }
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
    }
}
