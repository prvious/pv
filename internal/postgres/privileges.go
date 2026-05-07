package postgres

import (
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"syscall"
)

// dropCredential returns the credential pv should drop to when launching
// postgres binaries (initdb, postgres). Returns nil when no drop is needed
// (running as a non-root user, the typical dev case).
//
// When running as root with SUDO_UID/SUDO_GID set in the environment
// (which is what `sudo -E` populates), returns those — postgres refuses
// to run as root and forces us to drop, but the daemon itself often needs
// root to bind :443. This is the reconciliation.
func dropCredential() *syscall.Credential {
	if os.Geteuid() != 0 {
		return nil
	}
	uidStr := os.Getenv("SUDO_UID")
	gidStr := os.Getenv("SUDO_GID")
	if uidStr == "" || gidStr == "" {
		return nil
	}
	uid, err := strconv.ParseUint(uidStr, 10, 32)
	if err != nil {
		return nil
	}
	gid, err := strconv.ParseUint(gidStr, 10, 32)
	if err != nil {
		return nil
	}
	return &syscall.Credential{Uid: uint32(uid), Gid: uint32(gid)}
}

// dropSysProcAttr wraps dropCredential into a SysProcAttr suitable for
// exec.Cmd.SysProcAttr or supervisor.Process.SysProcAttr. Returns nil when
// no drop is needed.
func dropSysProcAttr() *syscall.SysProcAttr {
	cred := dropCredential()
	if cred == nil {
		return nil
	}
	return &syscall.SysProcAttr{Credential: cred}
}

// chownToTarget recursively chowns path to the SUDO_UID/SUDO_GID when
// running as root. No-op when running as a non-root user. Used so that
// initdb and postgres (which we drop to a non-root user) can read+write
// the data dir + binary tree that pv created while running as root.
func chownToTarget(path string) error {
	cred := dropCredential()
	if cred == nil {
		return nil
	}
	uid := int(cred.Uid)
	gid := int(cred.Gid)
	return filepath.Walk(path, func(p string, info os.FileInfo, err error) error {
		if err != nil {
			return err
		}
		// Use Lchown so symlinks themselves get chowned, not their targets.
		if err := os.Lchown(p, uid, gid); err != nil {
			return fmt.Errorf("chown %s: %w", p, err)
		}
		return nil
	})
}
