package project

import (
	"bufio"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

const ContractVersion = 1

type Contract struct {
	Version  int
	PHP      string
	Hosts    []string
	Services []string
	Setup    []string
}

func DefaultLaravelContract(projectName string) Contract {
	host := defaultHostLabel(projectName)
	return Contract{
		Version:  ContractVersion,
		PHP:      "8.4",
		Hosts:    []string{host + ".test"},
		Services: []string{"mailpit"},
		Setup:    []string{"composer install", "php artisan key:generate --ansi"},
	}
}

func defaultHostLabel(projectName string) string {
	var b strings.Builder
	previousHyphen := false
	for _, r := range strings.ToLower(strings.TrimSpace(projectName)) {
		if (r >= 'a' && r <= 'z') || (r >= '0' && r <= '9') {
			b.WriteRune(r)
			previousHyphen = false
			continue
		}
		if b.Len() > 0 && !previousHyphen {
			b.WriteByte('-')
			previousHyphen = true
		}
	}
	label := strings.Trim(b.String(), "-")
	if len(label) > 63 {
		label = strings.TrimRight(label[:63], "-")
	}
	if label == "" {
		return "app"
	}
	return label
}

func DetectLaravel(dir string) bool {
	if _, err := os.Stat(filepath.Join(dir, "artisan")); err == nil {
		return true
	}
	data, err := os.ReadFile(filepath.Join(dir, "composer.json"))
	return err == nil && strings.Contains(string(data), "laravel/framework")
}

func WriteContract(dir string, contract Contract, force bool) error {
	if err := contract.Validate(); err != nil {
		return err
	}
	path := filepath.Join(dir, "pv.yml")
	if !force {
		if _, err := os.Stat(path); err == nil {
			return fmt.Errorf("pv.yml already exists: use --force to overwrite")
		} else if !errors.Is(err, os.ErrNotExist) {
			return err
		}
	}
	return os.WriteFile(path, []byte(contract.String()), 0o644)
}

func LoadContract(dir string) (Contract, error) {
	data, err := os.ReadFile(filepath.Join(dir, "pv.yml"))
	if err != nil {
		return Contract{}, err
	}
	return ParseContract(string(data))
}

func ParseContract(data string) (Contract, error) {
	var contract Contract
	var section string
	scanner := bufio.NewScanner(strings.NewReader(data))
	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		if strings.HasSuffix(line, ":") {
			section = strings.TrimSuffix(line, ":")
			continue
		}
		if strings.HasPrefix(line, "- ") {
			value := strings.TrimSpace(strings.TrimPrefix(line, "- "))
			switch section {
			case "hosts":
				contract.Hosts = append(contract.Hosts, value)
			case "services":
				contract.Services = append(contract.Services, value)
			case "setup":
				contract.Setup = append(contract.Setup, value)
			default:
				return Contract{}, fmt.Errorf("list item outside supported section %q", section)
			}
			continue
		}
		key, value, ok := strings.Cut(line, ":")
		if !ok {
			return Contract{}, fmt.Errorf("invalid contract line %q", line)
		}
		section = ""
		switch strings.TrimSpace(key) {
		case "version":
			if strings.TrimSpace(value) != "1" {
				return Contract{}, fmt.Errorf("unsupported pv.yml version %q", strings.TrimSpace(value))
			}
			contract.Version = ContractVersion
		case "php":
			contract.PHP = strings.TrimSpace(value)
		default:
			return Contract{}, fmt.Errorf("unsupported pv.yml field %q", strings.TrimSpace(key))
		}
	}
	if err := scanner.Err(); err != nil {
		return Contract{}, err
	}
	return contract, contract.Validate()
}

func (c Contract) Validate() error {
	if c.Version != ContractVersion {
		return fmt.Errorf("pv.yml version must be %d", ContractVersion)
	}
	if c.PHP == "" {
		return errors.New("php version is required")
	}
	if len(c.Hosts) == 0 {
		return errors.New("at least one host is required")
	}
	for _, command := range c.Setup {
		if strings.TrimSpace(command) == "" {
			return errors.New("setup commands must not be empty")
		}
	}
	return nil
}

func (c Contract) String() string {
	var b strings.Builder
	fmt.Fprintf(&b, "version: %d\n", c.Version)
	fmt.Fprintf(&b, "php: %s\n", c.PHP)
	writeList(&b, "hosts", c.Hosts)
	writeList(&b, "services", c.Services)
	writeList(&b, "setup", c.Setup)
	return b.String()
}

func writeList(b *strings.Builder, name string, values []string) {
	fmt.Fprintf(b, "%s:\n", name)
	for _, value := range values {
		fmt.Fprintf(b, "  - %s\n", value)
	}
}
