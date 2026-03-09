package certs

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/pem"
	"math/big"
	"os"
	"path/filepath"
	"testing"
	"time"
)

func createTestCA(t *testing.T, dir string) (certPath, keyPath string) {
	t.Helper()

	caKey, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		t.Fatalf("generate CA key: %v", err)
	}

	template := &x509.Certificate{
		SerialNumber:          big.NewInt(1),
		Subject:               pkix.Name{CommonName: "Test CA"},
		NotBefore:             time.Now().Add(-time.Hour),
		NotAfter:              time.Now().Add(24 * time.Hour),
		IsCA:                  true,
		BasicConstraintsValid: true,
		KeyUsage:              x509.KeyUsageCertSign,
	}

	certDER, err := x509.CreateCertificate(rand.Reader, template, template, &caKey.PublicKey, caKey)
	if err != nil {
		t.Fatalf("create CA cert: %v", err)
	}

	certPath = filepath.Join(dir, "root.crt")
	certPEM := pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: certDER})
	if err := os.WriteFile(certPath, certPEM, 0644); err != nil {
		t.Fatalf("write CA cert: %v", err)
	}

	keyPath = filepath.Join(dir, "root.key")
	keyDER, err := x509.MarshalECPrivateKey(caKey)
	if err != nil {
		t.Fatalf("marshal CA key: %v", err)
	}
	keyPEM := pem.EncodeToMemory(&pem.Block{Type: "EC PRIVATE KEY", Bytes: keyDER})
	if err := os.WriteFile(keyPath, keyPEM, 0600); err != nil {
		t.Fatalf("write CA key: %v", err)
	}

	return certPath, keyPath
}

func TestGenerateSiteCert(t *testing.T) {
	dir := t.TempDir()
	caCertPath, caKeyPath := createTestCA(t, dir)

	certPath := filepath.Join(dir, "myapp.test.crt")
	keyPath := filepath.Join(dir, "myapp.test.key")

	if err := GenerateSiteCert("myapp.test", caCertPath, caKeyPath, certPath, keyPath); err != nil {
		t.Fatalf("GenerateSiteCert() error = %v", err)
	}

	// Verify cert file exists and is valid.
	certPEM, err := os.ReadFile(certPath)
	if err != nil {
		t.Fatalf("read cert: %v", err)
	}
	block, _ := pem.Decode(certPEM)
	if block == nil {
		t.Fatal("no PEM block in cert")
	}
	cert, err := x509.ParseCertificate(block.Bytes)
	if err != nil {
		t.Fatalf("parse cert: %v", err)
	}

	if cert.Subject.CommonName != "myapp.test" {
		t.Errorf("CommonName = %q, want %q", cert.Subject.CommonName, "myapp.test")
	}
	if len(cert.DNSNames) != 1 || cert.DNSNames[0] != "myapp.test" {
		t.Errorf("DNSNames = %v, want [myapp.test]", cert.DNSNames)
	}
	if cert.NotAfter.Before(time.Now().Add(800 * 24 * time.Hour)) {
		t.Error("cert expires too soon")
	}

	// Verify key file exists and is valid.
	keyPEM, err := os.ReadFile(keyPath)
	if err != nil {
		t.Fatalf("read key: %v", err)
	}
	keyBlock, _ := pem.Decode(keyPEM)
	if keyBlock == nil {
		t.Fatal("no PEM block in key")
	}

	// Verify key permissions.
	info, err := os.Stat(keyPath)
	if err != nil {
		t.Fatalf("stat key: %v", err)
	}
	if info.Mode().Perm() != 0600 {
		t.Errorf("key permissions = %o, want 0600", info.Mode().Perm())
	}

	// Verify the cert is signed by the CA.
	caCertPEM, _ := os.ReadFile(caCertPath)
	caBlock, _ := pem.Decode(caCertPEM)
	caCert, _ := x509.ParseCertificate(caBlock.Bytes)

	pool := x509.NewCertPool()
	pool.AddCert(caCert)

	if _, err := cert.Verify(x509.VerifyOptions{
		Roots:     pool,
		DNSName:   "myapp.test",
		KeyUsages: []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth},
	}); err != nil {
		t.Errorf("cert verification failed: %v", err)
	}
}

func TestGenerateSiteCert_MissingCA(t *testing.T) {
	dir := t.TempDir()

	err := GenerateSiteCert("myapp.test",
		filepath.Join(dir, "nonexistent.crt"),
		filepath.Join(dir, "nonexistent.key"),
		filepath.Join(dir, "out.crt"),
		filepath.Join(dir, "out.key"),
	)
	if err == nil {
		t.Fatal("expected error for missing CA")
	}
}
