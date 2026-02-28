package server

import (
	"fmt"
	"net"
	"testing"
	"time"

	"github.com/miekg/dns"
)

func startTestDNS(t *testing.T, tld string) (*DNSServer, string) {
	t.Helper()

	// Find a free UDP port.
	conn, err := net.ListenPacket("udp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("cannot find free port: %v", err)
	}
	addr := conn.LocalAddr().String()
	conn.Close()

	srv := NewDNSServer(tld)
	srv.Addr = addr

	errCh := make(chan error, 1)
	go func() { errCh <- srv.Start() }()

	// Wait for the server to be ready.
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		c := new(dns.Client)
		c.Timeout = 200 * time.Millisecond
		m := new(dns.Msg)
		m.SetQuestion("probe.test.", dns.TypeA)
		_, _, err := c.Exchange(m, addr)
		if err == nil {
			t.Cleanup(func() { srv.Shutdown() })
			return srv, addr
		}
		time.Sleep(50 * time.Millisecond)
	}

	select {
	case err := <-errCh:
		t.Fatalf("DNS server exited early: %v", err)
	default:
	}
	t.Fatal("DNS server did not become ready")
	return nil, ""
}

func TestDNSServer_ResolvesTestDomain(t *testing.T) {
	_, addr := startTestDNS(t, "test")

	c := new(dns.Client)
	m := new(dns.Msg)
	m.SetQuestion("myapp.test.", dns.TypeA)

	r, _, err := c.Exchange(m, addr)
	if err != nil {
		t.Fatalf("DNS query failed: %v", err)
	}

	if len(r.Answer) != 1 {
		t.Fatalf("expected 1 answer, got %d", len(r.Answer))
	}

	a, ok := r.Answer[0].(*dns.A)
	if !ok {
		t.Fatal("expected A record")
	}
	if a.A.String() != "127.0.0.1" {
		t.Errorf("got %s, want 127.0.0.1", a.A.String())
	}
}

func TestDNSServer_ResolvesSubdomain(t *testing.T) {
	_, addr := startTestDNS(t, "test")

	c := new(dns.Client)
	m := new(dns.Msg)
	m.SetQuestion("sub.myapp.test.", dns.TypeA)

	r, _, err := c.Exchange(m, addr)
	if err != nil {
		t.Fatalf("DNS query failed: %v", err)
	}

	if len(r.Answer) != 1 {
		t.Fatalf("expected 1 answer, got %d", len(r.Answer))
	}

	a := r.Answer[0].(*dns.A)
	if a.A.String() != "127.0.0.1" {
		t.Errorf("got %s, want 127.0.0.1", a.A.String())
	}
}

func TestDNSServer_CustomTLD(t *testing.T) {
	_, addr := startTestDNS(t, "pv-test")

	c := new(dns.Client)
	m := new(dns.Msg)
	m.SetQuestion("mysite.pv-test.", dns.TypeA)

	r, _, err := c.Exchange(m, addr)
	if err != nil {
		t.Fatalf("DNS query failed: %v", err)
	}

	if len(r.Answer) != 1 {
		t.Fatalf("expected 1 answer, got %d", len(r.Answer))
	}
}

func TestDNSServer_IgnoresNonAQuery(t *testing.T) {
	_, addr := startTestDNS(t, "test")

	c := new(dns.Client)
	m := new(dns.Msg)
	m.SetQuestion("myapp.test.", dns.TypeAAAA)

	r, _, err := c.Exchange(m, addr)
	if err != nil {
		t.Fatalf("DNS query failed: %v", err)
	}

	if len(r.Answer) != 0 {
		t.Errorf("expected 0 answers for AAAA query, got %d", len(r.Answer))
	}
}

func TestDNSServer_Shutdown(t *testing.T) {
	conn, err := net.ListenPacket("udp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("cannot find free port: %v", err)
	}
	addr := conn.LocalAddr().String()
	conn.Close()

	srv := NewDNSServer("test")
	srv.Addr = addr

	errCh := make(chan error, 1)
	go func() { errCh <- srv.Start() }()

	// Wait for server to start.
	time.Sleep(100 * time.Millisecond)

	if err := srv.Shutdown(); err != nil {
		t.Fatalf("Shutdown() error = %v", err)
	}

	select {
	case err := <-errCh:
		if err != nil {
			t.Logf("Start() returned: %v (expected for shutdown)", err)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("Start() did not return after Shutdown()")
	}

	// Verify server is no longer responding.
	c := new(dns.Client)
	c.Timeout = 500 * time.Millisecond
	m := new(dns.Msg)
	m.SetQuestion("myapp.test.", dns.TypeA)
	_, _, err = c.Exchange(m, addr)
	if err == nil {
		t.Error("expected error after shutdown, got nil")
	}
}

func TestNewDNSServer_DefaultAddr(t *testing.T) {
	srv := NewDNSServer("test")
	expected := fmt.Sprintf("127.0.0.1:%d", 10053)
	if srv.Addr != expected {
		t.Errorf("Addr = %q, want %q", srv.Addr, expected)
	}
}
