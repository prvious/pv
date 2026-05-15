package server

import (
	"fmt"
	"net"
	"os"

	"github.com/miekg/dns"
	"github.com/prvious/pv/internal/config"
)

// DNSServer resolves *.{tld} queries to 127.0.0.1.
type DNSServer struct {
	tld    string
	Addr   string // listen address, default "127.0.0.1:{DNSPort}"
	server *dns.Server
	ready  chan struct{}
}

// NewDNSServer creates a DNS server for the given TLD.
func NewDNSServer(tld string) *DNSServer {
	return &DNSServer{
		tld:   tld,
		Addr:  fmt.Sprintf("127.0.0.1:%d", config.DNSPort),
		ready: make(chan struct{}),
	}
}

// Ready returns a channel that is closed when the server has bound its port.
func (d *DNSServer) Ready() <-chan struct{} {
	return d.ready
}

// Start begins serving DNS queries. It blocks until Shutdown is called.
func (d *DNSServer) Start() error {
	mux := dns.NewServeMux()
	mux.HandleFunc(d.tld+".", d.handleQuery)

	d.server = &dns.Server{
		Addr:              d.Addr,
		Net:               "udp",
		Handler:           mux,
		NotifyStartedFunc: func() { close(d.ready) },
	}
	return d.server.ListenAndServe()
}

// Shutdown stops the DNS server.
func (d *DNSServer) Shutdown() error {
	if d.server == nil {
		return nil
	}
	return d.server.Shutdown()
}

func (d *DNSServer) handleQuery(w dns.ResponseWriter, r *dns.Msg) {
	msg := new(dns.Msg)
	msg.SetReply(r)
	msg.Authoritative = true

	for _, q := range r.Question {
		switch q.Qtype {
		case dns.TypeA:
			msg.Answer = append(msg.Answer, &dns.A{
				Hdr: dns.RR_Header{
					Name:   q.Name,
					Rrtype: dns.TypeA,
					Class:  dns.ClassINET,
					Ttl:    60,
				},
				A: net.ParseIP("127.0.0.1"),
			})
		case dns.TypeAAAA:
			msg.Answer = append(msg.Answer, &dns.AAAA{
				Hdr: dns.RR_Header{
					Name:   q.Name,
					Rrtype: dns.TypeAAAA,
					Class:  dns.ClassINET,
					Ttl:    60,
				},
				AAAA: net.ParseIP("::1"),
			})
		}
	}

	if err := w.WriteMsg(msg); err != nil {
		qname := "(unknown)"
		if len(r.Question) > 0 {
			qname = r.Question[0].Name
		}
		fmt.Fprintf(os.Stderr, "DNS: failed to write response for %s: %v\n", qname, err)
	}
}
