package project

import "fmt"

func Artisan(contract Contract, args ...string) []string {
	return append([]string{"php", "artisan"}, args...)
}

func DB(contract Contract, args ...string) ([]string, error) {
	if !hasService(contract, "postgres") && !hasService(contract, "mysql") {
		return nil, fmt.Errorf("project does not declare a database resource")
	}
	return append([]string{"db"}, args...), nil
}

func Mail(contract Contract, args ...string) ([]string, error) {
	if !hasService(contract, "mailpit") {
		return nil, fmt.Errorf("project does not declare mailpit")
	}
	return append([]string{"mail"}, args...), nil
}

func S3(contract Contract, args ...string) ([]string, error) {
	if !hasService(contract, "rustfs") {
		return nil, fmt.Errorf("project does not declare rustfs")
	}
	return append([]string{"s3"}, args...), nil
}

func hasService(contract Contract, service string) bool {
	for _, candidate := range contract.Services {
		if candidate == service {
			return true
		}
	}
	return false
}
