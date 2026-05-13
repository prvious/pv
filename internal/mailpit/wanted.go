package mailpit

func WantedVersions() ([]string, error) {
	st, err := LoadState()
	if err != nil {
		return nil, err
	}
	var versions []string
	for version, entry := range st.Versions {
		if entry.Wanted == WantedRunning && IsInstalled(version) {
			versions = append(versions, version)
		}
	}
	return versions, nil
}
