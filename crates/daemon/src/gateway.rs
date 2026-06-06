use std::collections::{BTreeMap, btree_map};
use std::net::TcpListener;

use camino::Utf8PathBuf;
use config::ProjectConfigFile;
use resources::{ArtifactManifestCache, ResourceName, TrackSelector};
use state::{
    Database, PortRequest, PvPaths, RUNTIME_PORT_FALLBACK_END, RUNTIME_PORT_FALLBACK_START,
};

use crate::DaemonError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimePlan {
    pub gateway: GatewayRuntimePlan,
    pub workers: Vec<PhpWorkerRuntimePlan>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GatewayRuntimePlan {
    pub http_port: u16,
    pub https_port: u16,
    pub ca_certificate_path: Utf8PathBuf,
    pub ca_private_key_path: Utf8PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhpWorkerRuntimePlan {
    pub php_track: String,
    pub port: u16,
    pub projects: Vec<RuntimeProject>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeProject {
    pub id: String,
    pub primary_hostname: String,
    pub hostnames: Vec<String>,
    pub project_root: Utf8PathBuf,
    pub document_root: Utf8PathBuf,
}

pub fn build_runtime_plan(paths: &PvPaths) -> Result<RuntimePlan, DaemonError> {
    let mut database = Database::open(paths)?;
    let gateway_ports = database.assign_gateway_ports(local_loopback_port_available)?;
    let mut projects_by_php_track: BTreeMap<String, PhpWorkerRuntimePlan> = BTreeMap::new();

    for project in database.projects()? {
        let config_file = ProjectConfigFile::read_from_root(&project.path)?;
        let php_track = resolve_project_php_track(
            paths,
            config_file.config.php.as_deref(),
            project.desired_php_track.as_deref(),
        )?;
        let document_root = match config_file.config.document_root {
            Some(document_root) => project.path.join(document_root),
            None => project.path.clone(),
        };
        let runtime_project = RuntimeProject {
            id: project.id,
            primary_hostname: project.primary_hostname.clone(),
            hostnames: additional_hostnames(
                &project.primary_hostname,
                project.additional_hostnames,
                config_file.config.hostnames,
            ),
            project_root: project.path,
            document_root,
        };

        match projects_by_php_track.entry(php_track.clone()) {
            btree_map::Entry::Occupied(mut entry) => {
                entry.get_mut().projects.push(runtime_project);
            }
            btree_map::Entry::Vacant(entry) => {
                let assignment = database.assign_port(
                    PortRequest::php_worker(
                        &php_track,
                        RUNTIME_PORT_FALLBACK_START,
                        RUNTIME_PORT_FALLBACK_START,
                        RUNTIME_PORT_FALLBACK_END,
                    ),
                    local_loopback_port_available,
                )?;

                entry.insert(PhpWorkerRuntimePlan {
                    php_track,
                    port: assignment.port,
                    projects: vec![runtime_project],
                });
            }
        }
    }

    let workers = projects_by_php_track
        .into_values()
        .map(|mut worker| {
            worker
                .projects
                .sort_by(|left, right| left.primary_hostname.cmp(&right.primary_hostname));
            worker
        })
        .collect();

    Ok(RuntimePlan {
        gateway: GatewayRuntimePlan {
            http_port: gateway_ports.http.port,
            https_port: gateway_ports.https.port,
            ca_certificate_path: paths.ca_certificate(),
            ca_private_key_path: paths.ca_private_key(),
        },
        workers,
    })
}

fn resolve_project_php_track(
    paths: &PvPaths,
    config_selector: Option<&str>,
    stored_selector: Option<&str>,
) -> Result<String, DaemonError> {
    let selector = config_selector
        .or(stored_selector)
        .map(TrackSelector::parse)
        .transpose()?
        .unwrap_or(TrackSelector::Latest);

    match selector {
        TrackSelector::Latest => latest_php_track(paths),
        TrackSelector::Track(track) => Ok(track.as_str().to_owned()),
    }
}

fn latest_php_track(paths: &PvPaths) -> Result<String, DaemonError> {
    let manifest = ArtifactManifestCache::new(paths.downloads().to_path_buf()).load_cached()?;
    let php = ResourceName::new("php")?;
    let track = manifest.resolve_track(&php, TrackSelector::Latest)?;

    Ok(track.as_str().to_owned())
}

fn additional_hostnames(
    primary_hostname: &str,
    state_hostnames: Vec<String>,
    config_hostnames: Vec<String>,
) -> Vec<String> {
    let mut hostnames = state_hostnames
        .into_iter()
        .chain(config_hostnames)
        .filter(|hostname| hostname != primary_hostname)
        .collect::<Vec<_>>();

    hostnames.sort();
    hostnames.dedup();
    hostnames
}

fn local_loopback_port_available(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}
