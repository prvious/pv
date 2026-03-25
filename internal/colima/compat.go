package colima

import (
	"fmt"
	"os"
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
		fmt.Fprintf(os.Stderr, "Warning: could not detect macOS version for VZ compatibility check: %v\n", err)
		return nil
	}

	major, err := parseMajorVersion(ver)
	if err != nil {
		fmt.Fprintf(os.Stderr, "Warning: could not parse macOS version %q for VZ compatibility check: %v\n", ver, err)
		return nil
	}

	if major < minMacOSMajor {
		return fmt.Errorf("colima requires macOS %d+ (Ventura) for the Virtualization framework, detected macOS %s", minMacOSMajor, ver)
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
