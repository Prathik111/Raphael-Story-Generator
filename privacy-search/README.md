# Raphael Private Web Research Gateway

This directory provides the privacy boundary used by Raphael Story Generator.

Traffic flow:

~~~text
Raphael Story Generator
        |
        | localhost only
        v
   SearXNG :8080
        |
        | SOCKS5h
        v
      Tor :9050
        |
        v
Internet search engines
and source websites
~~~

The application is configured to reject public SearXNG instances. It also fails closed when private research is enabled without a local proxy.

SearXNG is configured to send its outbound engine requests through Tor, and Raphael fetches source pages through the local Tor SOCKS5h proxy as well. SearXNG documents that self-hosted instances avoid having to trust a third-party instance administrator and can be configured to use Tor; it also recommends socks5h for Tor so DNS resolution happens through the proxy.

## Start

Install Docker Desktop, then from this directory run:

~~~powershell
$env:SEARXNG_SECRET = [guid]::NewGuid().ToString("N")
docker compose up -d --build
~~~

Verify:

~~~powershell
curl.exe -X POST "http://127.0.0.1:8080/search" -d "q=Raphael%20Story%20Generator&format=json"
~~~

Keep both services bound to localhost. Do not expose port 8080 or 9050 to the LAN or Internet.

For stronger anonymity, Tor should remain the only outbound path for both SearXNG and source fetching. Search providers and source websites will then see the Tor network rather than the user's normal public IP.

## Raphael settings

The desktop application defaults to:

- Search gateway: http://127.0.0.1:8080
- SOCKS proxy: socks5h://127.0.0.1:9050
- Require proxy: enabled
- Public search gateways: rejected

Do not change these to a hosted/public SearXNG URL when privacy is required.
