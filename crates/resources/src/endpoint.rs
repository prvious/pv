pub const ARTIFACT_MANIFEST_URL_BUILD_ENV: &str = "PV_DEFAULT_ARTIFACT_MANIFEST_URL";
pub const STABLE_ARTIFACT_MANIFEST_URL: &str = "https://artifacts.prvious.test/manifest.json";

pub fn default_artifact_manifest_url() -> &'static str {
    option_env!("PV_DEFAULT_ARTIFACT_MANIFEST_URL").unwrap_or(STABLE_ARTIFACT_MANIFEST_URL)
}
