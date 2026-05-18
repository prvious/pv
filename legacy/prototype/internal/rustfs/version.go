package rustfs

import "fmt"

const defaultVersion = "1.0.0-beta"

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
		return fmt.Errorf("rustfs: unsupported version %q (only %q is currently supported)", version, DefaultVersion())
	}
	return nil
}
