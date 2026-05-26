use crate::ResourceName;
use crate::error::{ResourcesError, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceKind {
    Runtime,
    Tool,
    BackingService,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceCapability {
    Install,
    Init,
    Start,
    Stop,
    Readiness,
    Allocation,
    EnvValues,
    Logs,
    Commands,
}

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

pub fn all() -> &'static [ResourceDescriptor] {
    RESOURCES
}

pub fn get(name: &ResourceName) -> Result<&'static ResourceDescriptor> {
    canonical(name.as_str())
}

pub fn canonical(name: &str) -> Result<&'static ResourceDescriptor> {
    RESOURCES
        .iter()
        .find(|descriptor| descriptor.name == name)
        .ok_or_else(|| ResourcesError::UnknownResource {
            name: name.to_string(),
        })
}

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
    pub fn name(self) -> &'static str {
        self.name
    }

    pub fn aliases(self) -> &'static [&'static str] {
        self.aliases
    }

    pub fn kind(self) -> ResourceKind {
        self.kind
    }

    pub fn capabilities(self) -> &'static [ResourceCapability] {
        self.capabilities
    }

    pub fn is_alias(self, value: &str) -> bool {
        self.aliases.contains(&value)
    }

    pub fn is_canonical(self, value: &str) -> bool {
        self.name == value
    }
}
