FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends tor ca-certificates \
    && rm -rf /var/lib/apt/lists/*

EXPOSE 9050

USER debian-tor

CMD ["tor", "--SocksPort", "0.0.0.0:9050", "--CookieAuthentication", "0", "--Log", "notice", "stdout"]
