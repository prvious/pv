package phpenv

import (
	"fmt"
	"io"
	"os"
	"path/filepath"

	"github.com/prvious/pv/internal/config"
)

// pvIniHeader is the header written into every conf.d/00-pv.ini.
const pvIniHeader = `; Managed by pv — regenerated on every ` + "`pv php:install`" + ` / ` + "`pv php:update`" + `.
; For your own overrides, create a sibling file like 99-local.ini —
; conf.d files load alphabetically and later files win.
`

// EnsureIniLayout provisions the per-version ini directory layout under
// ~/.pv/php/<version>/. It is idempotent and safe to call repeatedly:
//
//   - Creates etc/, conf.d/, ~/.pv/data/sessions/<version>/, and
//     ~/.pv/data/tmp/<version>/.
//   - If etc/php.ini does not exist AND etc/php.ini-development exists,
//     copies the latter to the former. Existing etc/php.ini is preserved.
//   - Always (re)writes conf.d/00-pv.ini with pv's path defaults for the
//     given version. This file is pv-managed.
func EnsureIniLayout(version string) error {
	dirs := []string{
		config.PhpEtcDir(version),
		config.PhpConfDDir(version),
		config.PhpSessionDir(version),
		config.PhpTmpDir(version),
	}
	for _, d := range dirs {
		if err := os.MkdirAll(d, 0755); err != nil {
			return fmt.Errorf("create %s: %w", d, err)
		}
	}

	if err := seedPhpIniIfMissing(version); err != nil {
		return err
	}

	return writePvIni(version)
}

// seedPhpIniIfMissing copies etc/php.ini-development to etc/php.ini if and
// only if php.ini does not yet exist. Missing source is a no-op (older
// artifacts didn't bundle the template).
func seedPhpIniIfMissing(version string) error {
	target := filepath.Join(config.PhpEtcDir(version), "php.ini")
	if _, err := os.Stat(target); err == nil {
		return nil // user file present; never touch.
	}

	source := filepath.Join(config.PhpEtcDir(version), "php.ini-development")
	if _, err := os.Stat(source); os.IsNotExist(err) {
		return nil // older artifact, nothing to copy.
	} else if err != nil {
		return fmt.Errorf("stat %s: %w", source, err)
	}

	in, err := os.Open(source)
	if err != nil {
		return fmt.Errorf("open %s: %w", source, err)
	}
	defer in.Close()

	out, err := os.OpenFile(target, os.O_CREATE|os.O_WRONLY|os.O_EXCL, 0644)
	if err != nil {
		return fmt.Errorf("create %s: %w", target, err)
	}
	if _, err := io.Copy(out, in); err != nil {
		out.Close()
		return fmt.Errorf("copy %s -> %s: %w", source, target, err)
	}
	return out.Close()
}

// writePvIni renders and writes conf.d/00-pv.ini. Always overwrites.
func writePvIni(version string) error {
	body := pvIniHeader + fmt.Sprintf(`
date.timezone = UTC

session.save_path = %q
sys_temp_dir     = %q
upload_tmp_dir   = %q
`,
		config.PhpSessionDir(version),
		config.PhpTmpDir(version),
		config.PhpTmpDir(version),
	)

	path := filepath.Join(config.PhpConfDDir(version), "00-pv.ini")
	if err := os.WriteFile(path, []byte(body), 0644); err != nil {
		return fmt.Errorf("write %s: %w", path, err)
	}
	return nil
}
