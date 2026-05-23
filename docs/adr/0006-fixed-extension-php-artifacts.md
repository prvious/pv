# Use fixed-extension PHP and FrankenPHP artifacts

PV v1 will avoid PHP extension management and instead distribute prebuilt macOS PHP and FrankenPHP artifacts with a common extension set baked in. This keeps local setup predictable and avoids building or configuring extensions per machine or per Project, at the cost of making unsupported extension needs outside v1 scope.
