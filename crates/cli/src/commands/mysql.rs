use std::process::ExitCode;

use crate::args::{ListArgs, MysqlInstallArgs, MysqlUninstallArgs};
use crate::environment::Environment;
use crate::error::ExecuteError;
use crate::output::Streams;

use super::artifact_resource::{self, ArtifactResourceCommandSpec};

pub(crate) fn install(
    args: MysqlInstallArgs,
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    artifact_resource::install(spec(), args.track.as_deref(), environment, streams)
}

pub(crate) fn update(
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    artifact_resource::update(spec(), environment, streams)
}

pub(crate) fn uninstall(
    args: MysqlUninstallArgs,
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    artifact_resource::uninstall(
        spec(),
        &args.track,
        args.prune,
        args.force,
        environment,
        streams,
    )
}

pub(crate) fn list(
    args: ListArgs,
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    artifact_resource::list(spec(), args, environment, streams)
}

fn spec() -> ArtifactResourceCommandSpec {
    ArtifactResourceCommandSpec {
        resource_name: "mysql",
        display_name: "MySQL",
        adapter: resources::mysql_adapter,
    }
}
