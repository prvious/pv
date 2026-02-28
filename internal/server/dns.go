package server

import (
	"fmt"
	"net"

	"github.com/miekg/dns"
	"github.com/prvious/pv/internal/config"
)

// DNSServer resolves *.{tld} queries to 127.0.0.1.
type DNSServer struct {
	tld    string
	Addr   string // listen address, default "127.0.0.1:{DNSPort}"
	server *dns.Server
}

// NewDNSServer creates a DNS server for the given TLD.
func NewDNSServer(tld string) *DNSServer {
	return &DNSServer{
		tld:  tld,
		Addr: fmt.Sprintf("127.0.0.1:%d", config.DNSPort),
	}
}

// Start begins serving DNS queries. It blocks until Shutdown is called.
func (d *DNSServer) Start() error {
	mux := dns.NewServeMux()
	mux.HandleFunc(d.tld+".", d.handleQuery)

	d.server = &dns.Server{
		Addr:    d.Addr,
		Net:     "udp",
		Handler: mux,
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
		if q.Qtype == dns.TypeA {
			msg.Answer = append(msg.Answer, &dns.A{
				Hdr: dns.RR_Header{
					Name:   q.Name,
					Rrtype: dns.TypeA,
					Class:  dns.ClassINET,
					Ttl:    60,
				},
				A: net.ParseIP("127.0.0.1"),
			})
		}
	}

	w.WriteMsg(msg)
}
