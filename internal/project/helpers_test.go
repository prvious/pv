package project

import "testing"

func TestHelpersRequireDeclaredResources(t *testing.T) {
	contract := Contract{Version: 1, PHP: "8.4", Hosts: []string{"app.test"}, Services: []string{"postgres", "mailpit", "rustfs"}}
	if got := Artisan(contract, "about"); got[0] != "php" || got[1] != "artisan" {
		t.Fatalf("artisan = %#v", got)
	}
	if _, err := DB(contract, "list"); err != nil {
		t.Fatalf("DB returned error: %v", err)
	}
	if _, err := Mail(contract, "open"); err != nil {
		t.Fatalf("Mail returned error: %v", err)
	}
	if _, err := S3(contract, "buckets"); err != nil {
		t.Fatalf("S3 returned error: %v", err)
	}
	if _, err := DB(Contract{Version: 1, PHP: "8.4", Hosts: []string{"app.test"}}, "list"); err == nil {
		t.Fatal("DB returned nil error for missing database")
	}
}
