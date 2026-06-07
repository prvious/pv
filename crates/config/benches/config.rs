#![allow(clippy::expect_used)]

use std::collections::BTreeMap;

use camino::Utf8Path;
use config::{
    AllocationEnvContext, ProjectConfig, ProjectEnvContext, ResourceEnvContext, format_env_value,
    format_project_env, hostname_from_project_path, normalize_primary_hostname, render_project_env,
};

fn main() {
    divan::main();
}

const SIMPLE_CONFIG: &str = r#"
php: 8.4
document_root: public
hostnames:
  - api.acme.test
"#;

const COMPLEX_CONFIG: &str = r#"
php: 8.4
document_root: public
hostnames:
  - api.acme.test
  - admin.acme.test
env:
  APP_URL: "${project_url}"
  APP_ENV: production
mysql:
  version: "8.4"
  env:
    DB_HOST: "${host}"
    DB_PORT: "${port}"
  allocations:
    app-db:
      env:
        DB_DATABASE: "${database}"
        DB_USERNAME: "${username}"
        DB_PASSWORD: "${password}"
rustfs:
  env:
    AWS_ENDPOINT: "${endpoint}"
    AWS_URL: "${url}"
  allocations:
    uploads:
      env:
        AWS_BUCKET: "${bucket}"
        AWS_ACCESS_KEY_ID: "${access_key}"
        AWS_SECRET_ACCESS_KEY: "${secret_key}"
postgresql:
  version: latest
"#;

#[divan::bench]
fn parse_simple() -> ProjectConfig {
    ProjectConfig::parse(divan::black_box(SIMPLE_CONFIG)).expect("simple config parses")
}

#[divan::bench]
fn parse_complex() -> ProjectConfig {
    ProjectConfig::parse(divan::black_box(COMPLEX_CONFIG)).expect("complex config parses")
}

fn values(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

fn render_inputs() -> (ProjectConfig, ProjectEnvContext) {
    let config = ProjectConfig::parse(COMPLEX_CONFIG).expect("complex config parses");

    let mut resources = BTreeMap::new();
    resources.insert(
        "mysql".to_string(),
        ResourceEnvContext {
            track: "8.4".to_string(),
            values: values(&[
                ("host", "127.0.0.1"),
                ("port", "3306"),
                ("password", "secret"),
                ("username", "root"),
            ]),
            allocations: BTreeMap::from([(
                "app-db".to_string(),
                AllocationEnvContext {
                    generated_name: "acme_test_app_db".to_string(),
                    values: values(&[
                        ("database", "acme_test_app_db"),
                        ("username", "root"),
                        ("password", "secret"),
                    ]),
                },
            )]),
        },
    );
    resources.insert(
        "rustfs".to_string(),
        ResourceEnvContext {
            track: "2026.1".to_string(),
            values: values(&[
                ("endpoint", "http://127.0.0.1:9000"),
                ("url", "http://127.0.0.1:9001"),
            ]),
            allocations: BTreeMap::from([(
                "uploads".to_string(),
                AllocationEnvContext {
                    generated_name: "acme-test-uploads".to_string(),
                    values: values(&[
                        ("bucket", "acme-test-uploads"),
                        ("access_key", "pv-access"),
                        ("secret_key", "pv-secret"),
                    ]),
                },
            )]),
        },
    );

    let context = ProjectEnvContext {
        primary_hostname: "acme.test".to_string(),
        resources,
    };

    (config, context)
}

#[divan::bench]
fn render_env(bencher: divan::Bencher) {
    let (config, context) = render_inputs();
    bencher.bench_local(|| {
        render_project_env(divan::black_box(&config), divan::black_box(&context))
            .expect("env renders")
    });
}

#[divan::bench]
fn format_env(bencher: divan::Bencher) {
    let (config, context) = render_inputs();
    let rendered = render_project_env(&config, &context).expect("env renders");
    bencher.bench_local(|| format_project_env(divan::black_box(&rendered)));
}

#[divan::bench]
fn format_value() -> String {
    format_env_value(divan::black_box(
        "value with \"quotes\", spaces and $special = chars",
    ))
}

#[divan::bench]
fn normalize_hostname() -> String {
    normalize_primary_hostname(divan::black_box("ApiAcmeExample")).expect("hostname normalizes")
}

#[divan::bench]
fn hostname_from_path() -> String {
    hostname_from_project_path(divan::black_box(Utf8Path::new(
        "/home/user/My Acme Project!",
    )))
    .expect("hostname derives from path")
}
