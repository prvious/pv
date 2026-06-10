use std::io::Write;
use std::process::ExitCode;

use crate::args::{ListArgs, PostgresInstallArgs, PostgresUninstallArgs};
use crate::commands::artifact_resource::{self, ArtifactResourceCommandSpec};
use crate::environment::Environment;
use crate::error::ExecuteError;

const SPEC: ArtifactResourceCommandSpec = ArtifactResourceCommandSpec {
    resource_name: "postgres",
    display_name: "Postgres",
    adapter: resources::postgres_adapter,
};

pub(crate) fn install(
    args: PostgresInstallArgs,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    artifact_resource::install(SPEC, args.track.as_deref(), environment, stdout)
}

pub(crate) fn update(
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    artifact_resource::update(SPEC, environment, stdout)
}

pub(crate) fn uninstall(
    args: PostgresUninstallArgs,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    artifact_resource::uninstall(
        SPEC,
        &args.track,
        args.prune,
        args.force,
        environment,
        stdout,
    )
}

pub(crate) fn list(
    args: ListArgs,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    artifact_resource::list(SPEC, args, environment, stdout)
}
