//! Fixtures shared by the root integration test targets.

use std::process::Output;

use anyhow::Result;
use assert_cmd::Command;
use camino::Utf8Path;

/// Runs the built `pv` in `current_dir` with `home` as its home, without a
/// terminal.
pub(crate) fn run_pv_in(args: &[&str], current_dir: &Utf8Path, home: &Utf8Path) -> Result<Output> {
    let mut command = Command::cargo_bin("pv")?;
    command.env_remove("NO_COLOR");
    command.env("HOME", home.as_str());
    command.current_dir(current_dir);

    Ok(command.args(args).output()?)
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI integration tests create fixture directories"
)]
pub(crate) fn create_dir(path: &Utf8Path) -> Result<()> {
    std::fs::create_dir_all(path)?;

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI integration tests write fixture files"
)]
pub(crate) fn write_file(path: &Utf8Path, contents: &str) -> Result<()> {
    std::fs::write(path, contents)?;

    Ok(())
}

/// A minimal Laravel Project with Vite and Managed Resource env keys.
pub(crate) fn create_laravel_init_fixture(project: &Utf8Path) -> Result<()> {
    create_dir(&project.join("bootstrap"))?;
    create_dir(&project.join("config"))?;
    create_dir(&project.join("public"))?;
    write_file(&project.join("artisan"), "")?;
    write_file(&project.join("bootstrap/app.php"), "<?php\n")?;
    write_file(&project.join("config/app.php"), "<?php\n")?;
    write_file(&project.join("public/index.php"), "<?php\n")?;
    write_file(
        &project.join("composer.json"),
        r#"{"require":{"php":"^8.4","laravel/framework":"^12.0"}}"#,
    )?;
    write_file(
        &project.join("package.json"),
        r#"{"devDependencies":{"vite":"^7.0.0","laravel-vite-plugin":"^2.0.0"}}"#,
    )?;
    write_file(
        &project.join(".env.example"),
        r#"APP_URL=http://localhost
DB_CONNECTION=mysql
REDIS_HOST=127.0.0.1
CACHE_STORE=redis
MAIL_MAILER=smtp
AWS_ACCESS_KEY_ID=
AWS_SECRET_ACCESS_KEY=
"#,
    )?;

    Ok(())
}
