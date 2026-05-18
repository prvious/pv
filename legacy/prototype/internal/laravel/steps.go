package laravel

// isLaravel returns true if the project type is Laravel or Laravel with Octane.
func isLaravel(projectType string) bool {
	return projectType == "laravel" || projectType == "laravel-octane"
}
