# Use Gateway-managed certificates for explicit Project hostnames

PV will use its local CA with the Gateway's FrankenPHP/Caddy configuration and let FrankenPHP/Caddy generate Project certificates as needed instead of PV pre-generating certificates or using one wildcard `*.test` certificate. Certificates are only for the Project's primary hostname and any additional hostnames explicitly requested in Project config, so Project subdomains can be routed deliberately while the Gateway still centralizes TLS termination and SNI selection.
