pub const ARTIFACT_MANIFEST_URL_BUILD_ENV: &str = "PV_DEFAULT_ARTIFACT_MANIFEST_URL";
pub const STABLE_ARTIFACT_MANIFEST_URL: &str = "https://artifacts.prvious.test/manifest.json";

pub fn default_artifact_manifest_url() -> &'static str {
    option_env!("PV_DEFAULT_ARTIFACT_MANIFEST_URL").unwrap_or(STABLE_ARTIFACT_MANIFEST_URL)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_artifact_manifest_url_matches_compiled_value() {
        let expected =
            option_env!("PV_DEFAULT_ARTIFACT_MANIFEST_URL").unwrap_or(STABLE_ARTIFACT_MANIFEST_URL);

        assert_eq!(default_artifact_manifest_url(), expected);
    }

    #[test]
    fn stable_artifact_manifest_url_is_the_current_default_endpoint() {
        assert_eq!(
            STABLE_ARTIFACT_MANIFEST_URL,
            "https://artifacts.prvious.test/manifest.json"
        );
    }
}
