use std::io::Write;
use std::process::ExitCode;

use crate::args::{ListArgs, RedisInstallArgs, RedisUninstallArgs};
use crate::commands::artifact_resource::{self, ArtifactResourceCommandSpec};
use crate::environment::Environment;
use crate::error::ExecuteError;

const SPEC: ArtifactResourceCommandSpec = ArtifactResourceCommandSpec {
    resource_name: "redis",
    display_name: "Redis",
    adapter: resources::redis_adapter,
};

pub(crate) fn install(
    args: RedisInstallArgs,
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
    args: RedisUninstallArgs,
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
