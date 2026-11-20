# Verifying a change over HTTPS

PV serves each Project at `https://<hostname>` with a certificate from its own local certificate authority (CA). Setup trusts that CA in the operating system's store. Every one-label subdomain, such as `https://tenant.acme.test`, reaches the same Project.

Never turn off certificate verification (`curl -k`, `verify: false`, `NODE_TLS_REJECT_UNAUTHORIZED=0`) to make a check pass. Point the client at PV's CA instead:

```
~/.pv/certificates/ca.pem
```

## By client

| Client | What to do |
| --- | --- |
| `curl` on macOS (`/usr/bin/curl`) | Works as is; it reads the system keychain. |
| PHP run by PV | Pass the CA: `CURLOPT_CAINFO`, the stream context's `ssl.cafile`, or in Laravel `Http::withOptions(['verify' => getenv('HOME').'/.pv/certificates/ca.pem'])`. This replaces PHP's default roots, so use it only for `.test` requests. |
| Node | `NODE_EXTRA_CA_CERTS=~/.pv/certificates/ca.pem node …` adds the CA to Node's roots. |
| Chrome and Chromium-based browsers, including Playwright's Chromium and Chrome DevTools | Trust the CA through the system store on macOS and the NSS database on Linux. |

For other tools, look for a CA bundle setting such as `SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE` or `--cacert`.

## Inside an agent sandbox

Sandboxes often block requests to `.test` hostnames, because they resolve to `127.0.0.1`:

- Codex's default sandbox blocks loopback and local addresses.
- Claude Code's sandbox proxy answers with `403` and `resolved to a loopback address` unless `sandbox.network.allowedDomains` lists both `<hostname>:443` and `127.0.0.1:443`.
- Cursor's sandbox blocks `127.x` addresses by default.

These responses come from the sandbox, not from the application. Report them to the developer, or ask to run the check outside the sandbox. Don't change the application to work around them.

## Reading the response

- A failed TLS handshake, such as curl's `tlsv1 alert internal error`, usually means no Project is linked at that hostname, so PV has no certificate for it. Look the hostname up with `pv list --json`.
- An untrusted-certificate error from a client that reads the system store means setup's CA trust needs repair. `pv doctor` names the fix.
- A `5xx` error comes from the application. Check its logs, such as `storage/logs/laravel.log`, and `pv logs --worker <php-track>`.
