use std::io::Write;
use std::process::ExitCode;

use crate::args::{ListArgs, MysqlInstallArgs, MysqlUninstallArgs};
use crate::environment::Environment;
use crate::error::ExecuteError;

use super::artifact_resource::{self, ArtifactResourceCommandSpec};

pub(crate) fn install(
    args: MysqlInstallArgs,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    artifact_resource::install(spec(), args.track.as_deref(), environment, stdout)
}

pub(crate) fn update(
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    artifact_resource::update(spec(), environment, stdout)
}

pub(crate) fn uninstall(
    args: MysqlUninstallArgs,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    artifact_resource::uninstall(
        spec(),
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
    artifact_resource::list(spec(), args, environment, stdout)
}

fn spec() -> ArtifactResourceCommandSpec {
    ArtifactResourceCommandSpec {
        resource_name: "mysql",
        display_name: "MySQL",
        adapter: resources::mysql_adapter,
    }
}
