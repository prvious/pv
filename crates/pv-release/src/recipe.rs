use camino::{Utf8Path, Utf8PathBuf};
use resources::{
    ArtifactPlatform, PvVersion, ResourceName, ResourcesError, Sha256Digest, TrackName,
};
use serde::Deserialize;
use std::collections::BTreeSet;
use url::Url;

const REQUIRED_PHP_EXTENSIONS: &[&str] = &[
    "bcmath",
    "ctype",
    "curl",
    "dom",
    "fileinfo",
    "filter",
    "hash",
    "iconv",
    "intl",
    "json",
    "libxml",
    "mbstring",
    "openssl",
    "pcntl",
    "pcre",
    "pdo",
    "pdo_mysql",
    "pdo_pgsql",
    "pdo_sqlite",
    "phar",
    "posix",
    "redis",
    "session",
    "simplexml",
    "sodium",
    "sqlite3",
    "tokenizer",
    "xml",
    "xmlreader",
    "xmlwriter",
    "zip",
    "zlib",
];

const PHP_DEPLOYMENT_TARGET: &str = "13.0";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackingRecipeKind {
    Redis,
    Mysql,
    Postgres,
    Mailpit,
    Rustfs,
}

#[derive(Clone, Debug)]
pub struct BackingRecipe {
    path: Utf8PathBuf,
    kind: BackingRecipeKind,
    resource: ResourceName,
    header: RecipeHeader,
    artifact: BackingArtifact,
    tracks: Vec<BackingTrack>,
}

#[derive(Clone, Debug)]
pub struct BackingArtifact {
    payload_paths: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct BackingTrack {
    name: TrackName,
    upstream_version: String,
    source_url: String,
    source_sha256: Sha256Digest,
}

#[derive(Clone, Debug)]
pub struct PhpRecipe {
    path: Utf8PathBuf,
    header: RecipeHeader,
    php: PhpSettings,
    frankenphp: FrankenphpSettings,
    tracks: Vec<PhpTrack>,
}

#[derive(Clone, Debug)]
pub struct RecipeHeader {
    resources: Vec<ResourceName>,
    default_track: TrackName,
    platforms: Vec<ArtifactPlatform>,
    minimum_pv_version: PvVersion,
    pv_build_revision: String,
    license_files: Vec<String>,
    notice_files: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct PhpSettings {
    deployment_target: String,
    build_extensions: Vec<String>,
    expected_extensions: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct FrankenphpSettings {
    version: String,
    source_url: String,
    source_sha256: Sha256Digest,
}

#[derive(Clone, Debug)]
pub struct PhpTrack {
    name: TrackName,
    php_version: String,
    php_source_url: String,
    php_source_sha256: Sha256Digest,
}

#[derive(Clone, Debug)]
pub struct ComposerRecipe {
    path: Utf8PathBuf,
    header: RecipeHeader,
    track: ComposerTrack,
    platform: ArtifactPlatform,
}

#[derive(Clone, Debug)]
pub struct ComposerTrack {
    name: TrackName,
    upstream_version: String,
    source_url: String,
    source_sha256: Sha256Digest,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPhpRecipe {
    recipe: RawRecipeHeader,
    php: RawPhpSettings,
    frankenphp: RawFrankenphpSettings,
    #[serde(default)]
    tracks: Vec<RawPhpTrack>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawComposerRecipe {
    recipe: RawRecipeHeader,
    #[serde(default)]
    tracks: Vec<RawComposerTrack>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBackingRecipe {
    recipe: RawRecipeHeader,
    artifact: RawBackingArtifact,
    #[serde(default)]
    tracks: Vec<RawBackingTrack>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRecipeHeader {
    resources: Vec<String>,
    default_track: String,
    platforms: Vec<String>,
    minimum_pv_version: String,
    pv_build_revision: String,
    license_files: Vec<String>,
    #[serde(default)]
    notice_files: Vec<String>,
}

#[derive(Clone, Copy)]
enum LegalFilePolicy {
    ExactStandard,
    StandardPlusAdditional,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPhpSettings {
    deployment_target: String,
    build_extensions: Vec<String>,
    expected_extensions: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFrankenphpSettings {
    version: String,
    source_url: String,
    source_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPhpTrack {
    name: String,
    php_version: String,
    php_source_url: String,
    php_source_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawComposerTrack {
    name: String,
    upstream_version: String,
    source_url: String,
    source_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBackingArtifact {
    payload_paths: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBackingTrack {
    name: String,
    upstream_version: String,
    source_url: String,
    source_sha256: String,
}

impl BackingRecipeKind {
    pub fn resource_name(self) -> &'static str {
        match self {
            Self::Redis => "redis",
            Self::Mysql => "mysql",
            Self::Postgres => "postgres",
            Self::Mailpit => "mailpit",
            Self::Rustfs => "rustfs",
        }
    }

    fn recipe_kind(self) -> &'static str {
        match self {
            Self::Redis => "Redis recipe",
            Self::Mysql => "MySQL recipe",
            Self::Postgres => "Postgres recipe",
            Self::Mailpit => "Mailpit recipe",
            Self::Rustfs => "RustFS recipe",
        }
    }
}

impl BackingRecipe {
    pub fn from_toml(
        path: &Utf8Path,
        kind: BackingRecipeKind,
        content: &str,
    ) -> crate::Result<Self> {
        let raw: RawBackingRecipe =
            toml::from_str(content).map_err(|error| invalid(path, error))?;
        Self::from_raw(path, kind, raw)
    }

    pub fn load(path: &Utf8Path, kind: BackingRecipeKind) -> crate::Result<Self> {
        let content = read_to_string(path)?;
        Self::from_toml(path, kind, &content)
    }

    pub fn kind(&self) -> BackingRecipeKind {
        self.kind
    }

    pub fn resource(&self) -> &ResourceName {
        &self.resource
    }

    pub fn tracks(&self) -> &[BackingTrack] {
        &self.tracks
    }

    pub fn path(&self) -> &Utf8Path {
        &self.path
    }

    pub fn platforms(&self) -> &[ArtifactPlatform] {
        &self.header.platforms
    }

    pub fn default_track(&self) -> &TrackName {
        &self.header.default_track
    }

    pub fn minimum_pv_version(&self) -> &PvVersion {
        &self.header.minimum_pv_version
    }

    pub fn pv_build_revision(&self) -> &str {
        &self.header.pv_build_revision
    }

    pub fn license_files(&self) -> &[String] {
        &self.header.license_files
    }

    pub fn notice_files(&self) -> &[String] {
        &self.header.notice_files
    }

    pub fn payload_paths(&self) -> &[String] {
        &self.artifact.payload_paths
    }

    fn from_raw(
        path: &Utf8Path,
        kind: BackingRecipeKind,
        raw: RawBackingRecipe,
    ) -> crate::Result<Self> {
        let header =
            RecipeHeader::from_raw(path, raw.recipe, LegalFilePolicy::StandardPlusAdditional)?;
        let resource = ResourceName::new(kind.resource_name())
            .map_err(|error| invalid_identity(path, "resource", error))?;
        validate_exact_resources(
            path,
            kind.recipe_kind(),
            &header.resources,
            &[kind.resource_name()],
        )?;
        validate_exact_platforms(
            path,
            kind.recipe_kind(),
            &header.platforms,
            &["darwin-arm64", "darwin-amd64"],
        )?;

        let artifact = BackingArtifact::from_raw(path, raw.artifact)?;
        let tracks = parse_backing_tracks(path, raw.tracks)?;
        validate_default_track_exists(
            path,
            &header.default_track,
            tracks.iter().map(BackingTrack::name),
        )?;

        Ok(Self {
            path: path.to_path_buf(),
            kind,
            resource,
            header,
            artifact,
            tracks,
        })
    }
}

impl BackingArtifact {
    fn from_raw(path: &Utf8Path, raw: RawBackingArtifact) -> crate::Result<Self> {
        if raw.payload_paths.is_empty() {
            return Err(invalid(path, "artifact.payload_paths must not be empty"));
        }
        validate_relative_file_list(path, "artifact.payload_paths", &raw.payload_paths)?;

        Ok(Self {
            payload_paths: raw.payload_paths,
        })
    }
}

impl BackingTrack {
    pub fn name(&self) -> &TrackName {
        &self.name
    }

    pub fn upstream_version(&self) -> &str {
        &self.upstream_version
    }

    pub fn source_url(&self) -> &str {
        &self.source_url
    }

    pub fn source_sha256(&self) -> &Sha256Digest {
        &self.source_sha256
    }

    fn from_raw(path: &Utf8Path, name: TrackName, raw: RawBackingTrack) -> crate::Result<Self> {
        let upstream_version =
            require_non_empty(path, "upstream_version", &raw.upstream_version)?.to_string();
        let source_url = parse_https_url(path, "source_url", raw.source_url)?;
        let source_sha256 = Sha256Digest::new(raw.source_sha256)
            .map_err(|error| invalid_identity(path, "source_sha256", error))?;

        Ok(Self {
            name,
            upstream_version,
            source_url,
            source_sha256,
        })
    }
}

impl PhpRecipe {
    pub fn from_toml(path: &Utf8Path, content: &str) -> crate::Result<Self> {
        let raw: RawPhpRecipe = toml::from_str(content).map_err(|error| invalid(path, error))?;
        Self::from_raw(path, raw)
    }

    pub fn load(path: &Utf8Path) -> crate::Result<Self> {
        let content = read_to_string(path)?;
        Self::from_toml(path, &content)
    }

    pub fn tracks(&self) -> &[PhpTrack] {
        &self.tracks
    }

    pub fn path(&self) -> &Utf8Path {
        &self.path
    }

    pub fn platforms(&self) -> &[ArtifactPlatform] {
        &self.header.platforms
    }

    pub fn default_track(&self) -> &TrackName {
        &self.header.default_track
    }

    pub fn minimum_pv_version(&self) -> &PvVersion {
        &self.header.minimum_pv_version
    }

    pub fn pv_build_revision(&self) -> &str {
        &self.header.pv_build_revision
    }

    pub fn license_files(&self) -> &[String] {
        &self.header.license_files
    }

    pub fn notice_files(&self) -> &[String] {
        &self.header.notice_files
    }

    pub fn deployment_target(&self) -> &str {
        &self.php.deployment_target
    }

    pub fn build_extensions(&self) -> &[String] {
        &self.php.build_extensions
    }

    pub fn expected_extensions(&self) -> &[String] {
        &self.php.expected_extensions
    }

    pub fn frankenphp_version(&self) -> &str {
        &self.frankenphp.version
    }

    pub fn frankenphp_source_url(&self) -> &str {
        &self.frankenphp.source_url
    }

    pub fn frankenphp_source_sha256(&self) -> &Sha256Digest {
        &self.frankenphp.source_sha256
    }

    fn from_raw(path: &Utf8Path, raw: RawPhpRecipe) -> crate::Result<Self> {
        let header = RecipeHeader::from_raw(path, raw.recipe, LegalFilePolicy::ExactStandard)?;
        validate_exact_resources(
            path,
            "PHP recipe",
            &header.resources,
            &["frankenphp", "php"],
        )?;
        validate_exact_platforms(
            path,
            "PHP recipe",
            &header.platforms,
            &["darwin-amd64", "darwin-arm64"],
        )?;

        let php = PhpSettings::from_raw(path, raw.php)?;
        let frankenphp = FrankenphpSettings::from_raw(path, raw.frankenphp)?;
        let tracks = parse_php_tracks(path, raw.tracks)?;
        validate_default_track_exists(
            path,
            &header.default_track,
            tracks.iter().map(PhpTrack::name),
        )?;

        Ok(Self {
            path: path.to_path_buf(),
            header,
            php,
            frankenphp,
            tracks,
        })
    }
}

impl RecipeHeader {
    fn from_raw(
        path: &Utf8Path,
        raw: RawRecipeHeader,
        legal_file_policy: LegalFilePolicy,
    ) -> crate::Result<Self> {
        let resources = parse_resource_list(path, raw.resources)?;
        let default_track = TrackName::new(raw.default_track)
            .map_err(|error| invalid_identity(path, "default_track", error))?;
        let platforms = parse_platform_list(path, raw.platforms)?;
        let minimum_pv_version = PvVersion::parse(raw.minimum_pv_version)
            .map_err(|error| invalid_identity(path, "minimum_pv_version", error))?;
        let pv_build_revision =
            require_non_empty(path, "pv_build_revision", &raw.pv_build_revision)?.to_string();
        validate_relative_file_list(path, "license_files", &raw.license_files)?;
        validate_relative_file_list(path, "notice_files", &raw.notice_files)?;
        match legal_file_policy {
            LegalFilePolicy::ExactStandard => {
                validate_exact_file_list(path, "license_files", &raw.license_files, &["LICENSE"])?;
                validate_exact_file_list(path, "notice_files", &raw.notice_files, &["NOTICE"])?;
            }
            LegalFilePolicy::StandardPlusAdditional => {
                validate_required_file(path, "license_files", &raw.license_files, "LICENSE")?;
                validate_required_file(path, "notice_files", &raw.notice_files, "NOTICE")?;
            }
        }

        Ok(Self {
            resources,
            default_track,
            platforms,
            minimum_pv_version,
            pv_build_revision,
            license_files: raw.license_files,
            notice_files: raw.notice_files,
        })
    }
}

impl PhpSettings {
    fn from_raw(path: &Utf8Path, raw: RawPhpSettings) -> crate::Result<Self> {
        validate_deployment_target(path, &raw.deployment_target)?;
        validate_expected_extensions(path, &raw.expected_extensions)?;
        validate_build_extensions(path, &raw.build_extensions, &raw.expected_extensions)?;

        Ok(Self {
            deployment_target: raw.deployment_target,
            build_extensions: raw.build_extensions,
            expected_extensions: raw.expected_extensions,
        })
    }
}

impl FrankenphpSettings {
    fn from_raw(path: &Utf8Path, raw: RawFrankenphpSettings) -> crate::Result<Self> {
        let version = require_non_empty(path, "frankenphp.version", &raw.version)?.to_string();
        let source_url = parse_https_url(path, "frankenphp.source_url", raw.source_url)?;
        let source_sha256 = Sha256Digest::new(raw.source_sha256)
            .map_err(|error| invalid_identity(path, "frankenphp.source_sha256", error))?;

        Ok(Self {
            version,
            source_url,
            source_sha256,
        })
    }
}

impl PhpTrack {
    pub fn name(&self) -> &TrackName {
        &self.name
    }

    pub fn php_version(&self) -> &str {
        &self.php_version
    }

    pub fn php_source_url(&self) -> &str {
        &self.php_source_url
    }

    pub fn php_source_sha256(&self) -> &Sha256Digest {
        &self.php_source_sha256
    }

    fn from_raw(path: &Utf8Path, name: TrackName, raw: RawPhpTrack) -> crate::Result<Self> {
        let php_version = require_non_empty(path, "php_version", &raw.php_version)?.to_string();
        validate_php_track_version(path, &name, &php_version)?;
        let php_source_url = parse_https_url(path, "php_source_url", raw.php_source_url)?;
        let php_source_sha256 = Sha256Digest::new(raw.php_source_sha256)
            .map_err(|error| invalid_identity(path, "php_source_sha256", error))?;

        Ok(Self {
            name,
            php_version,
            php_source_url,
            php_source_sha256,
        })
    }
}

impl ComposerRecipe {
    pub fn from_toml(path: &Utf8Path, content: &str) -> crate::Result<Self> {
        let raw: RawComposerRecipe =
            toml::from_str(content).map_err(|error| invalid(path, error))?;
        Self::from_raw(path, raw)
    }

    pub fn load(path: &Utf8Path) -> crate::Result<Self> {
        let content = read_to_string(path)?;
        Self::from_toml(path, &content)
    }

    pub fn track(&self) -> &TrackName {
        &self.track.name
    }

    pub fn path(&self) -> &Utf8Path {
        &self.path
    }

    pub fn upstream_version(&self) -> &str {
        &self.track.upstream_version
    }

    pub fn platform(&self) -> ArtifactPlatform {
        self.platform
    }

    pub fn minimum_pv_version(&self) -> &PvVersion {
        &self.header.minimum_pv_version
    }

    pub fn pv_build_revision(&self) -> &str {
        &self.header.pv_build_revision
    }

    pub fn license_files(&self) -> &[String] {
        &self.header.license_files
    }

    pub fn notice_files(&self) -> &[String] {
        &self.header.notice_files
    }

    pub fn source_url(&self) -> &str {
        &self.track.source_url
    }

    pub fn source_sha256(&self) -> &Sha256Digest {
        &self.track.source_sha256
    }

    fn from_raw(path: &Utf8Path, raw: RawComposerRecipe) -> crate::Result<Self> {
        let header = RecipeHeader::from_raw(path, raw.recipe, LegalFilePolicy::ExactStandard)?;
        validate_exact_resources(path, "Composer recipe", &header.resources, &["composer"])?;
        validate_exact_platforms(path, "Composer recipe", &header.platforms, &["any"])?;

        let [raw_track] = raw
            .tracks
            .try_into()
            .map_err(|tracks: Vec<RawComposerTrack>| {
                invalid(
                    path,
                    format!(
                        "Composer recipe must contain exactly one track, got {}",
                        tracks.len()
                    ),
                )
            })?;
        let track = ComposerTrack::from_raw(path, raw_track)?;
        if track.name.as_str() != "2" {
            return Err(invalid(path, "Composer recipe track must be `2`"));
        }
        validate_default_track_exists(path, &header.default_track, std::iter::once(&track.name))?;

        Ok(Self {
            path: path.to_path_buf(),
            header,
            track,
            platform: ArtifactPlatform::Any,
        })
    }
}

impl ComposerTrack {
    fn from_raw(path: &Utf8Path, raw: RawComposerTrack) -> crate::Result<Self> {
        let name =
            TrackName::new(raw.name).map_err(|error| invalid_identity(path, "track", error))?;
        let upstream_version =
            require_non_empty(path, "upstream_version", &raw.upstream_version)?.to_string();
        let source_url = parse_https_url(path, "source_url", raw.source_url)?;
        let source_sha256 = Sha256Digest::new(raw.source_sha256)
            .map_err(|error| invalid_identity(path, "source_sha256", error))?;

        Ok(Self {
            name,
            upstream_version,
            source_url,
            source_sha256,
        })
    }
}

pub fn composer_recipe_env(
    composer: &Utf8Path,
    resource: &str,
    track: &str,
    platform: &str,
) -> crate::Result<String> {
    let recipe = ComposerRecipe::load(composer)?;
    validate_composer_recipe_request(&recipe, resource, track, platform)?;

    let upstream_version = recipe.upstream_version();
    let pv_build_revision = recipe.pv_build_revision();
    let artifact_version = format!("{upstream_version}-{pv_build_revision}");
    let source_url = recipe.source_url();
    let source_sha256 = recipe.source_sha256().as_str();
    let minimum_pv_version = recipe.minimum_pv_version().as_str();

    shell_assignments(
        recipe.path(),
        &[
            ("PV_RESOURCE", "resource", "composer"),
            ("PV_TRACK", "track", "2"),
            ("PV_PLATFORM", "platform", "any"),
            ("PV_UPSTREAM_VERSION", "upstream_version", upstream_version),
            (
                "PV_ARTIFACT_VERSION",
                "artifact_version",
                artifact_version.as_str(),
            ),
            ("PV_SOURCE_URL", "source_url", source_url),
            ("PV_SOURCE_SHA256", "source_sha256", source_sha256),
            (
                "PV_MINIMUM_PV_VERSION",
                "minimum_pv_version",
                minimum_pv_version,
            ),
            (
                "PV_PV_BUILD_REVISION",
                "pv_build_revision",
                pv_build_revision,
            ),
        ],
    )
}

pub fn backing_recipe_env(
    backing: &Utf8Path,
    kind: BackingRecipeKind,
    resource: &str,
    track: &str,
    platform: &str,
) -> crate::Result<String> {
    let recipe = BackingRecipe::load(backing, kind)?;
    validate_backing_recipe_resource(&recipe, resource)?;
    let track = validate_backing_recipe_track(&recipe, track)?;
    let platform = validate_backing_recipe_platform(&recipe, platform)?;

    let upstream_version = track.upstream_version();
    let pv_build_revision = recipe.pv_build_revision();
    let artifact_version = format!("{upstream_version}-{pv_build_revision}");
    let source_url = track.source_url();
    let source_sha256 = track.source_sha256().as_str();
    let minimum_pv_version = recipe.minimum_pv_version().as_str();

    shell_assignments(
        recipe.path(),
        &[
            ("PV_RESOURCE", "resource", recipe.resource().as_str()),
            ("PV_TRACK", "track", track.name().as_str()),
            ("PV_PLATFORM", "platform", platform.as_str()),
            ("PV_UPSTREAM_VERSION", "upstream_version", upstream_version),
            (
                "PV_ARTIFACT_VERSION",
                "artifact_version",
                artifact_version.as_str(),
            ),
            ("PV_SOURCE_URL", "source_url", source_url),
            ("PV_SOURCE_SHA256", "source_sha256", source_sha256),
            (
                "PV_MINIMUM_PV_VERSION",
                "minimum_pv_version",
                minimum_pv_version,
            ),
            (
                "PV_PV_BUILD_REVISION",
                "pv_build_revision",
                pv_build_revision,
            ),
        ],
    )
}

pub fn php_recipe_env(
    php: &Utf8Path,
    resource: &str,
    track: &str,
    platform: &str,
) -> crate::Result<String> {
    let recipe = PhpRecipe::load(php)?;
    let resource = validate_php_recipe_resource(&recipe, resource)?;
    let track = validate_php_recipe_track(&recipe, track)?;
    let platform = validate_php_recipe_platform(&recipe, platform)?;

    let php_version = track.php_version();
    let php_source_url = track.php_source_url();
    let php_source_sha256 = track.php_source_sha256().as_str();
    let source_url;
    let source_sha256;
    let upstream_version;
    match resource {
        PhpRecipeResource::Php => {
            upstream_version = php_version.to_string();
            source_url = php_source_url;
            source_sha256 = php_source_sha256;
        }
        PhpRecipeResource::Frankenphp => {
            upstream_version = format!("{php_version}-frankenphp{}", recipe.frankenphp_version());
            source_url = recipe.frankenphp_source_url();
            source_sha256 = recipe.frankenphp_source_sha256().as_str();
        }
    }

    let pv_build_revision = recipe.pv_build_revision();
    let artifact_version = format!("{upstream_version}-{pv_build_revision}");
    let build_extensions = recipe.build_extensions().join(",");
    let expected_extensions = recipe.expected_extensions().join(",");
    let minimum_pv_version = recipe.minimum_pv_version().as_str();
    let deployment_target = recipe.deployment_target();
    let mut assignments = vec![
        ("PV_RESOURCE", "resource", resource.as_str()),
        ("PV_TRACK", "track", track.name().as_str()),
        ("PV_PLATFORM", "platform", platform.as_str()),
        ("PV_PHP_VERSION", "php_version", php_version),
        (
            "PV_UPSTREAM_VERSION",
            "upstream_version",
            upstream_version.as_str(),
        ),
        (
            "PV_ARTIFACT_VERSION",
            "artifact_version",
            artifact_version.as_str(),
        ),
        ("PV_SOURCE_URL", "source_url", source_url),
        ("PV_SOURCE_SHA256", "source_sha256", source_sha256),
    ];

    if matches!(resource, PhpRecipeResource::Frankenphp) {
        assignments.extend([
            ("PV_PHP_SOURCE_URL", "php_source_url", php_source_url),
            (
                "PV_PHP_SOURCE_SHA256",
                "php_source_sha256",
                php_source_sha256,
            ),
        ]);
    }

    assignments.extend([
        (
            "PV_DEPLOYMENT_TARGET",
            "deployment_target",
            deployment_target,
        ),
        (
            "PV_BUILD_EXTENSIONS",
            "build_extensions",
            build_extensions.as_str(),
        ),
        (
            "PV_EXPECTED_EXTENSIONS",
            "expected_extensions",
            expected_extensions.as_str(),
        ),
        (
            "PV_MINIMUM_PV_VERSION",
            "minimum_pv_version",
            minimum_pv_version,
        ),
        (
            "PV_PV_BUILD_REVISION",
            "pv_build_revision",
            pv_build_revision,
        ),
    ]);

    shell_assignments(recipe.path(), &assignments)
}

fn shell_assignments(path: &Utf8Path, assignments: &[(&str, &str, &str)]) -> crate::Result<String> {
    let mut env = String::new();
    for (name, field, value) in assignments {
        env.push_str(name);
        env.push('=');
        env.push_str(&shell_quote_assignment_value(path, field, value)?);
        env.push('\n');
    }
    Ok(env)
}

fn shell_quote_assignment_value(
    path: &Utf8Path,
    field: &str,
    value: &str,
) -> crate::Result<String> {
    if value.contains('\n') || value.contains('\r') {
        return Err(invalid(
            path,
            format!("{field} contains a line break and cannot be written to recipe env output"),
        ));
    }

    Ok(format!("'{}'", value.replace('\'', "'\\''")))
}

#[derive(Clone, Copy)]
enum PhpRecipeResource {
    Php,
    Frankenphp,
}

impl PhpRecipeResource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Php => "php",
            Self::Frankenphp => "frankenphp",
        }
    }
}

fn validate_php_recipe_resource(
    recipe: &PhpRecipe,
    resource: &str,
) -> crate::Result<PhpRecipeResource> {
    match resource {
        "php" => Ok(PhpRecipeResource::Php),
        "frankenphp" => Ok(PhpRecipeResource::Frankenphp),
        _ => Err(invalid(
            recipe.path(),
            format!("PHP recipe resource must be `php` or `frankenphp`, got `{resource}`"),
        )),
    }
}

fn validate_php_recipe_track<'a>(
    recipe: &'a PhpRecipe,
    track: &str,
) -> crate::Result<&'a PhpTrack> {
    recipe
        .tracks()
        .iter()
        .find(|candidate| candidate.name().as_str() == track)
        .ok_or_else(|| {
            let expected = recipe
                .tracks()
                .iter()
                .map(|track| track.name().as_str())
                .collect::<BTreeSet<_>>();
            invalid(
                recipe.path(),
                format!(
                    "PHP recipe track must be one of {}, got `{track}`",
                    format_expected_values(&expected)
                ),
            )
        })
}

fn validate_php_recipe_platform(
    recipe: &PhpRecipe,
    platform: &str,
) -> crate::Result<ArtifactPlatform> {
    recipe
        .platforms()
        .iter()
        .copied()
        .find(|candidate| candidate.as_str() == platform)
        .ok_or_else(|| {
            let expected = recipe
                .platforms()
                .iter()
                .map(|platform| platform.as_str())
                .collect::<BTreeSet<_>>();
            invalid(
                recipe.path(),
                format!(
                    "PHP recipe platform must be one of {}, got `{platform}`",
                    format_expected_values(&expected)
                ),
            )
        })
}

fn validate_composer_recipe_request(
    recipe: &ComposerRecipe,
    resource: &str,
    track: &str,
    platform: &str,
) -> crate::Result<()> {
    validate_request_value(
        recipe.path(),
        "Composer recipe",
        "resource",
        resource,
        "composer",
    )?;
    validate_request_value(
        recipe.path(),
        "Composer recipe",
        "track",
        track,
        recipe.track().as_str(),
    )?;
    validate_request_value(
        recipe.path(),
        "Composer recipe",
        "platform",
        platform,
        recipe.platform().as_str(),
    )
}

fn validate_backing_recipe_resource(recipe: &BackingRecipe, resource: &str) -> crate::Result<()> {
    validate_request_value(
        recipe.path(),
        recipe.kind().recipe_kind(),
        "resource",
        resource,
        recipe.resource().as_str(),
    )
}

fn validate_backing_recipe_track<'a>(
    recipe: &'a BackingRecipe,
    track: &str,
) -> crate::Result<&'a BackingTrack> {
    recipe
        .tracks()
        .iter()
        .find(|candidate| candidate.name().as_str() == track)
        .ok_or_else(|| {
            let expected = recipe
                .tracks()
                .iter()
                .map(|track| track.name().as_str())
                .collect::<BTreeSet<_>>();
            invalid(
                recipe.path(),
                format!(
                    "{} track must be one of {}, got `{track}`",
                    recipe.kind().recipe_kind(),
                    format_expected_values(&expected)
                ),
            )
        })
}

fn validate_backing_recipe_platform(
    recipe: &BackingRecipe,
    platform: &str,
) -> crate::Result<ArtifactPlatform> {
    recipe
        .platforms()
        .iter()
        .copied()
        .find(|candidate| candidate.as_str() == platform)
        .ok_or_else(|| {
            let expected = recipe
                .platforms()
                .iter()
                .map(|platform| platform.as_str())
                .collect::<BTreeSet<_>>();
            invalid(
                recipe.path(),
                format!(
                    "{} platform must be one of {}, got `{platform}`",
                    recipe.kind().recipe_kind(),
                    format_expected_values(&expected)
                ),
            )
        })
}

fn validate_request_value(
    path: &Utf8Path,
    recipe_kind: &str,
    field: &str,
    actual: &str,
    expected: &str,
) -> crate::Result<()> {
    if actual == expected {
        Ok(())
    } else {
        Err(invalid(
            path,
            format!("{recipe_kind} {field} must be `{expected}`, got `{actual}`"),
        ))
    }
}

fn parse_resource_list(path: &Utf8Path, values: Vec<String>) -> crate::Result<Vec<ResourceName>> {
    if values.is_empty() {
        return Err(invalid(path, "recipe.resources must not be empty"));
    }

    let mut resources = Vec::with_capacity(values.len());
    let mut seen = BTreeSet::new();
    for value in values {
        let resource =
            ResourceName::new(value).map_err(|error| invalid_identity(path, "resource", error))?;
        if !seen.insert(resource.clone()) {
            return Err(invalid(path, format!("duplicate resource `{resource}`")));
        }
        resources.push(resource);
    }

    Ok(resources)
}

fn parse_platform_list(
    path: &Utf8Path,
    values: Vec<String>,
) -> crate::Result<Vec<ArtifactPlatform>> {
    if values.is_empty() {
        return Err(invalid(path, "recipe.platforms must not be empty"));
    }

    let mut platforms = Vec::with_capacity(values.len());
    let mut seen = BTreeSet::new();
    for value in values {
        let platform = ArtifactPlatform::new(&value)
            .map_err(|error| invalid_identity(path, "platform", error))?;
        if !seen.insert(platform) {
            return Err(invalid(path, format!("duplicate platform `{platform}`")));
        }
        platforms.push(platform);
    }

    Ok(platforms)
}

fn parse_php_tracks(path: &Utf8Path, values: Vec<RawPhpTrack>) -> crate::Result<Vec<PhpTrack>> {
    let mut names = Vec::with_capacity(values.len());
    let mut seen = BTreeSet::new();
    for value in &values {
        let name = TrackName::new(value.name.clone())
            .map_err(|error| invalid_identity(path, "track", error))?;
        if !seen.insert(name.clone()) {
            return Err(invalid(path, format!("duplicate track `{name}`")));
        }
        names.push(name);
    }

    let mut tracks = Vec::with_capacity(values.len());
    for (name, value) in names.into_iter().zip(values) {
        let track = PhpTrack::from_raw(path, name, value)?;
        tracks.push(track);
    }

    Ok(tracks)
}

fn parse_backing_tracks(
    path: &Utf8Path,
    values: Vec<RawBackingTrack>,
) -> crate::Result<Vec<BackingTrack>> {
    let mut names = Vec::with_capacity(values.len());
    let mut seen = BTreeSet::new();
    for value in &values {
        let name = TrackName::new(value.name.clone())
            .map_err(|error| invalid_identity(path, "track", error))?;
        if !seen.insert(name.clone()) {
            return Err(invalid(path, format!("duplicate track `{name}`")));
        }
        names.push(name);
    }

    let mut tracks = Vec::with_capacity(values.len());
    for (name, value) in names.into_iter().zip(values) {
        let track = BackingTrack::from_raw(path, name, value)?;
        tracks.push(track);
    }

    Ok(tracks)
}

fn validate_exact_resources(
    path: &Utf8Path,
    recipe_kind: &str,
    resources: &[ResourceName],
    expected: &[&str],
) -> crate::Result<()> {
    let actual: BTreeSet<&str> = resources.iter().map(ResourceName::as_str).collect();
    let expected: BTreeSet<&str> = expected.iter().copied().collect();

    if actual == expected {
        Ok(())
    } else {
        Err(invalid(
            path,
            format!(
                "{recipe_kind} resources must be exactly {}",
                format_expected_values(&expected)
            ),
        ))
    }
}

fn validate_exact_platforms(
    path: &Utf8Path,
    recipe_kind: &str,
    platforms: &[ArtifactPlatform],
    expected: &[&str],
) -> crate::Result<()> {
    let actual: BTreeSet<&str> = platforms.iter().map(|platform| platform.as_str()).collect();
    let expected: BTreeSet<&str> = expected.iter().copied().collect();

    if actual == expected {
        Ok(())
    } else {
        Err(invalid(
            path,
            format!(
                "{recipe_kind} platforms must be exactly {}",
                format_expected_values(&expected)
            ),
        ))
    }
}

fn validate_default_track_exists<'a>(
    path: &Utf8Path,
    default_track: &TrackName,
    tracks: impl Iterator<Item = &'a TrackName>,
) -> crate::Result<()> {
    if tracks.into_iter().any(|track| track == default_track) {
        Ok(())
    } else {
        Err(invalid(
            path,
            format!("recipe.default_track `{default_track}` must exist in tracks"),
        ))
    }
}

fn validate_php_track_version(
    path: &Utf8Path,
    track: &TrackName,
    php_version: &str,
) -> crate::Result<()> {
    let parts = php_version.split('.').collect::<Vec<_>>();
    let [major, minor, patch] = parts.as_slice() else {
        return Err(invalid(
            path,
            format!(
                "php_version `{php_version}` must be major.minor.patch with numeric components"
            ),
        ));
    };
    if [major, minor, patch].iter().any(|component| {
        component.is_empty()
            || !component
                .chars()
                .all(|character| character.is_ascii_digit())
    }) {
        return Err(invalid(
            path,
            format!(
                "php_version `{php_version}` must be major.minor.patch with numeric components"
            ),
        ));
    }

    let expected_track = format!("{major}.{minor}");
    if track.as_str() == expected_track {
        Ok(())
    } else {
        Err(invalid(
            path,
            format!("PHP track `{track}` must match php_version prefix `{expected_track}`"),
        ))
    }
}

fn validate_deployment_target(path: &Utf8Path, value: &str) -> crate::Result<()> {
    if value == PHP_DEPLOYMENT_TARGET {
        Ok(())
    } else {
        Err(invalid(
            path,
            format!("deployment_target must be `{PHP_DEPLOYMENT_TARGET}`"),
        ))
    }
}

fn validate_expected_extensions(path: &Utf8Path, values: &[String]) -> crate::Result<()> {
    let actual: BTreeSet<&str> = values.iter().map(String::as_str).collect();
    let missing = REQUIRED_PHP_EXTENSIONS
        .iter()
        .copied()
        .filter(|extension| !actual.contains(extension))
        .collect::<Vec<_>>();

    if !missing.is_empty() {
        return Err(invalid(
            path,
            format!(
                "expected_extensions missing required extensions: {}",
                missing.join(", ")
            ),
        ));
    }

    let required = REQUIRED_PHP_EXTENSIONS
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let unexpected = actual
        .iter()
        .copied()
        .filter(|extension| !required.contains(extension))
        .collect::<Vec<_>>();
    if unexpected.is_empty() {
        Ok(())
    } else {
        Err(invalid(
            path,
            format!(
                "expected_extensions contains unexpected extensions: {}",
                unexpected.join(", ")
            ),
        ))
    }
}

fn validate_build_extensions(
    path: &Utf8Path,
    build_extensions: &[String],
    expected_extensions: &[String],
) -> crate::Result<()> {
    if build_extensions.is_empty() {
        return Err(invalid(path, "build_extensions must not be empty"));
    }

    let expected: BTreeSet<&str> = expected_extensions.iter().map(String::as_str).collect();
    for extension in build_extensions {
        if !expected.contains(extension.as_str()) {
            return Err(invalid(
                path,
                format!(
                    "build_extensions contains extension `{extension}` outside expected_extensions"
                ),
            ));
        }
    }

    Ok(())
}

fn parse_https_url(path: &Utf8Path, field: &str, value: String) -> crate::Result<String> {
    let value = require_non_empty(path, field, &value)?.to_string();
    if value.contains('\\') {
        return Err(invalid(
            path,
            format!("{field} must be an https URL with a host"),
        ));
    }

    let parsed = Url::parse(&value)
        .map_err(|_error| invalid(path, format!("{field} must be an https URL with a host")))?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err(invalid(
            path,
            format!("{field} must be an https URL with a host"),
        ));
    }

    Ok(value)
}

fn validate_relative_file_list(
    path: &Utf8Path,
    field: &str,
    values: &[String],
) -> crate::Result<()> {
    for value in values {
        if !relative_path_is_valid(value) {
            return Err(invalid(
                path,
                format!("{field} contains invalid relative path `{value}`"),
            ));
        }
    }

    Ok(())
}

fn validate_exact_file_list(
    path: &Utf8Path,
    field: &str,
    values: &[String],
    expected: &[&str],
) -> crate::Result<()> {
    if values
        .iter()
        .map(String::as_str)
        .eq(expected.iter().copied())
    {
        return Ok(());
    }

    Err(invalid(
        path,
        format!(
            "{field} must be exactly {}",
            expected
                .iter()
                .map(|value| format!("`{value}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    ))
}

fn validate_required_file(
    path: &Utf8Path,
    field: &str,
    values: &[String],
    required: &str,
) -> crate::Result<()> {
    if values.iter().any(|value| value == required) {
        return Ok(());
    }

    Err(invalid(path, format!("{field} must include `{required}`")))
}

fn relative_path_is_valid(value: &str) -> bool {
    let candidate = Utf8Path::new(value);
    !candidate.is_absolute()
        && !value.is_empty()
        && !value.contains('\\')
        && !value.split('/').any(str::is_empty)
        && !candidate
            .components()
            .any(|component| matches!(component.as_str(), "." | ".."))
}

fn require_non_empty<'a>(path: &Utf8Path, field: &str, value: &'a str) -> crate::Result<&'a str> {
    if value.trim().is_empty() {
        Err(invalid(path, format!("{field} must not be empty")))
    } else {
        Ok(value)
    }
}

fn format_expected_values(values: &BTreeSet<&str>) -> String {
    values
        .iter()
        .map(|value| format!("`{value}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV release tooling reads repository-local recipe metadata"
)]
fn read_to_string(path: &Utf8Path) -> crate::Result<String> {
    std::fs::read_to_string(path).map_err(|error| invalid(path, error))
}

fn invalid(path: &Utf8Path, reason: impl ToString) -> crate::ReleaseError {
    crate::ReleaseError::InvalidRecipeMetadata {
        path: path.to_string(),
        reason: reason.to_string(),
    }
}

fn invalid_identity(path: &Utf8Path, field: &str, error: ResourcesError) -> crate::ReleaseError {
    crate::ReleaseError::InvalidRecipeMetadata {
        path: path.to_string(),
        reason: format!("invalid {field}: {error}"),
    }
}
