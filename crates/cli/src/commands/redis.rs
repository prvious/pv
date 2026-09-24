use std::process::ExitCode;

use crate::args::{ListArgs, RedisInstallArgs, RedisUninstallArgs};
use crate::commands::artifact_resource::{self, ArtifactResourceCommandSpec};
use crate::environment::Environment;
use crate::error::ExecuteError;
use crate::output::Streams;

const SPEC: ArtifactResourceCommandSpec = ArtifactResourceCommandSpec {
    resource_name: "redis",
    display_name: "Redis",
    adapter: resources::redis_adapter,
};

pub(crate) fn install(
    args: RedisInstallArgs,
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    artifact_resource::install(SPEC, args.track.as_deref(), environment, streams)
}

pub(crate) fn update(
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    artifact_resource::update(SPEC, environment, streams)
}

pub(crate) fn uninstall(
    args: RedisUninstallArgs,
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    artifact_resource::uninstall(
        SPEC,
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
    artifact_resource::list(SPEC, args, environment, streams)
}
