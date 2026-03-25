package colima

import (
	"fmt"
	"runtime"
	"strconv"
	"strings"
	"syscall"
)

const minMacOSMajor = 13 // Ventura, required for --vm-type vz and --mount-type virtiofs

// checkVZCompat verifies the system meets the minimum macOS version for the
// Virtualization framework (vz) backend. Returns nil on non-Darwin platforms
// or if the version cannot be determined (fail open).
func checkVZCompat() error {
	if runtime.GOOS != "darwin" {
		return nil
	}

	ver, err := syscall.Sysctl("kern.osproductversion")
	if err != nil {
		return nil
	}

	major, err := parseMajorVersion(ver)
	if err != nil {
		return nil
	}

	if major < minMacOSMajor {
		return fmt.Errorf("Colima requires macOS %d+ (Ventura) for the Virtualization framework, detected macOS %s", minMacOSMajor, ver)
	}
	return nil
}

func parseMajorVersion(ver string) (int, error) {
	parts := strings.SplitN(ver, ".", 2)
	if len(parts) == 0 {
		return 0, fmt.Errorf("empty version string")
	}
	return strconv.Atoi(parts[0])
}
