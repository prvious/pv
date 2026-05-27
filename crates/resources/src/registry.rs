use crate::error::{ResourcesError, Result};

/// Classifies a managed resource by its role in PV.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceKind {
    /// Execution environment used to run project workloads.
    Runtime,
    /// Developer-facing tool or command-line utility.
    Tool,
    /// Long-lived service that projects depend on.
    BackingService,
}

/// Describes operations supported by a managed resource.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceCapability {
    /// Resource can be installed from a managed artifact.
    Install,
    /// Resource has an initialization or configuration step.
    Init,
    /// Resource has start lifecycle behavior.
    Start,
    /// Resource has stop lifecycle behavior.
    Stop,
    /// Resource can report readiness after start.
    Readiness,
    /// Resource allocates per-project or per-track state.
    Allocation,
    /// Resource contributes environment values.
    EnvValues,
    /// Resource exposes logs.
    Logs,
    /// Resource exposes user-facing commands.
    Commands,
}

/// Static registry entry for a PV-managed resource.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceDescriptor {
    name: &'static str,
    aliases: &'static [&'static str],
    kind: ResourceKind,
    capabilities: &'static [ResourceCapability],
}

const RUNTIME_CAPABILITIES: &[ResourceCapability] = &[
    ResourceCapability::Install,
    ResourceCapability::Start,
    ResourceCapability::Stop,
    ResourceCapability::Readiness,
    ResourceCapability::Logs,
    ResourceCapability::Commands,
];

const TOOL_CAPABILITIES: &[ResourceCapability] =
    &[ResourceCapability::Install, ResourceCapability::Commands];

const BACKING_SERVICE_CAPABILITIES: &[ResourceCapability] = &[
    ResourceCapability::Install,
    ResourceCapability::Init,
    ResourceCapability::Start,
    ResourceCapability::Stop,
    ResourceCapability::Readiness,
    ResourceCapability::Allocation,
    ResourceCapability::EnvValues,
    ResourceCapability::Logs,
    ResourceCapability::Commands,
];

const NO_ALLOCATION_BACKING_SERVICE_CAPABILITIES: &[ResourceCapability] = &[
    ResourceCapability::Install,
    ResourceCapability::Init,
    ResourceCapability::Start,
    ResourceCapability::Stop,
    ResourceCapability::Readiness,
    ResourceCapability::EnvValues,
    ResourceCapability::Logs,
    ResourceCapability::Commands,
];

static RESOURCES: &[ResourceDescriptor] = &[
    ResourceDescriptor {
        name: "php",
        aliases: &[],
        kind: ResourceKind::Runtime,
        capabilities: RUNTIME_CAPABILITIES,
    },
    ResourceDescriptor {
        name: "frankenphp",
        aliases: &[],
        kind: ResourceKind::Runtime,
        capabilities: RUNTIME_CAPABILITIES,
    },
    ResourceDescriptor {
        name: "composer",
        aliases: &[],
        kind: ResourceKind::Tool,
        capabilities: TOOL_CAPABILITIES,
    },
    ResourceDescriptor {
        name: "mysql",
        aliases: &[],
        kind: ResourceKind::BackingService,
        capabilities: BACKING_SERVICE_CAPABILITIES,
    },
    ResourceDescriptor {
        name: "postgres",
        aliases: &["postgresql", "pg"],
        kind: ResourceKind::BackingService,
        capabilities: BACKING_SERVICE_CAPABILITIES,
    },
    ResourceDescriptor {
        name: "redis",
        aliases: &[],
        kind: ResourceKind::BackingService,
        capabilities: BACKING_SERVICE_CAPABILITIES,
    },
    ResourceDescriptor {
        name: "mailpit",
        aliases: &["mail"],
        kind: ResourceKind::BackingService,
        capabilities: NO_ALLOCATION_BACKING_SERVICE_CAPABILITIES,
    },
    ResourceDescriptor {
        name: "rustfs",
        aliases: &["s3"],
        kind: ResourceKind::BackingService,
        capabilities: BACKING_SERVICE_CAPABILITIES,
    },
];

/// Returns every statically registered resource descriptor.
pub fn all() -> &'static [ResourceDescriptor] {
    RESOURCES
}

/// Resolves a descriptor by canonical resource name only.
pub fn resolve_canonical(name: &str) -> Result<&'static ResourceDescriptor> {
    RESOURCES
        .iter()
        .find(|descriptor| descriptor.name == name)
        .ok_or_else(|| ResourcesError::UnknownResource {
            name: name.to_string(),
        })
}

/// Resolves a descriptor by canonical resource name or registered alias.
pub fn resolve(name_or_alias: &str) -> Result<&'static ResourceDescriptor> {
    RESOURCES
        .iter()
        .find(|descriptor| {
            descriptor.name == name_or_alias || descriptor.aliases.contains(&name_or_alias)
        })
        .ok_or_else(|| ResourcesError::UnknownResource {
            name: name_or_alias.to_string(),
        })
}

impl ResourceDescriptor {
    /// Returns the canonical resource name.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Returns alternate names accepted by alias-aware lookup.
    pub fn aliases(&self) -> &'static [&'static str] {
        self.aliases
    }

    /// Returns the resource category.
    pub fn kind(&self) -> ResourceKind {
        self.kind
    }

    /// Returns the supported resource operations.
    pub fn capabilities(&self) -> &'static [ResourceCapability] {
        self.capabilities
    }

    /// Returns true when the value is a registered alias.
    pub fn is_alias(&self, value: &str) -> bool {
        self.aliases.contains(&value)
    }

    /// Returns true when the value is the canonical resource name.
    pub fn is_canonical(&self, value: &str) -> bool {
        self.name == value
    }
}
