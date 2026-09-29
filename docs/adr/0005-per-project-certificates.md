# Use Gateway-managed certificates for Project hostnames

PV will use its local CA with the Gateway's FrankenPHP/Caddy configuration and let FrankenPHP/Caddy generate Project certificates as needed instead of PV pre-generating certificates or using one wildcard `*.test` certificate. Certificates are only for each served Project's primary hostname and its one-label wildcard, such as `acme.test` and `*.acme.test`, so Project subdomains are routed automatically while the Gateway still centralizes TLS termination and SNI selection.
