package mailpit

import "fmt"

const defaultVersion = "1"

func DefaultVersion() string { return defaultVersion }

func ResolveVersion(version string) (string, error) {
	if version == "" {
		return DefaultVersion(), nil
	}
	if err := ValidateVersion(version); err != nil {
		return "", err
	}
	return version, nil
}

func ValidateVersion(version string) error {
	if version != DefaultVersion() {
		return fmt.Errorf("mailpit: unsupported version %q (only %q is currently supported)", version, DefaultVersion())
	}
	return nil
}
