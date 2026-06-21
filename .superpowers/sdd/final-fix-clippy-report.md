# Final Clippy Fix Report

## Change

- Reduced `promote_runtime_config_tree` argument count by grouping its inputs into `RuntimeConfigTreePromotion`.
- Preserved the existing environment flow:
  - Gateway validation uses `frankenphp_xdg_environment(paths)`.
  - Worker validation uses `worker_config_private_environment(paths, &worker.php_track)`.
  - Worker startup remains on `frankenphp_worker_environment(paths, php_track)`.
- No Caddyfile `env` or `php_ini` rendering behavior was changed.

## Tests

No new tests were added because this is a behavior-preserving call-shape refactor for a clippy lint.

## Verification

```text
$ cargo fmt --all -- --check
exit code: 0
```

```text
$ cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
    Blocking waiting for file lock on build directory
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 20.81s
exit code: 0
```
