# lt-master

The lasertag server list. Game servers (dedicated ones and players' games hosting on the
internet) report to it every 30 seconds; games list them in the internet menu; a web browser
gets the list as a page. A server not heard of for 90 seconds is gone.

| Request | |
|---|---|
| `POST /api/servers` | A game server's report (`lt_master_proto::ServerReport`, JSON). Its address is where the request came from with the report's UDP port. |
| `DELETE /api/servers/<port>` | The game server at the request's address and that port goes. |
| `GET /api/servers` | The servers, most players first (`Vec<ServerEntry>`). |
| `GET /api/server/<id>` | One server by its `id` (a hash of its name and IP address). |
| `GET /` | The same as a web page. |
| `GET /health` | `{"ok":true}` |

## The browser relay

Browsers can't send UDP, so the browser build (`docs/web.md`) joins through the master server:
it opens a WebSocket at `/relay/<id>` (the `id` from the list), and the relay passes each
binary message on as one UDP datagram to that game server, from a socket of its own, and each
datagram back as one message. The game server sees an ordinary client at the master server's
address; game servers need nothing for it.

It relays only to servers on the list (not to ones on this machine or its LAN, unless
`--relay-internal`), only the game's packets (`LTAG` first), at most 4 relays per client
address and 512 in all, at most 300 packets a second from each browser, and closes a relay
after 15 seconds without packets. It listens on a port of its own (`--relay-bind`, default
`127.0.0.1:27501`, `off` for none); the proxy sends `/relay/*` there (`deploy/Caddyfile`).

The server speaks plain HTTP and keeps the list in memory (it fills again within half a
minute after a restart). HTTPS comes from a reverse proxy (Caddy, Cloudflare and cloudflared,
...); a request from this machine or its LAN is a proxy's, and the client is in its
`CF-Connecting-IP` or else the last internet address in `X-Forwarded-For`.

## Running it on lt-master.playspoon.com

Build it on its own, without the game's Bevy, audio and graphics dependencies
(`cargo metaserver` is an alias for `cargo build --release -p lt-master`):

```sh
cargo metaserver
sudo install target/release/lt-master /usr/local/bin/
sudo cp crates/lt-master/deploy/lt-master.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now lt-master
# Caddy for HTTPS (deploy/Caddyfile into /etc/caddy/Caddyfile, then reload).
sudo systemctl reload caddy
```

After an update, `install` the new binary again and `sudo systemctl restart lt-master`.
`journalctl -u lt-master -f` shows its log.

`lt-master --bind <addr:port> --threads <n> --relay-bind <addr:port>|off --relay-internal`
(default `0.0.0.0:27500`, 4, `127.0.0.1:27501`). The service binds `127.0.0.1:27500` and
`127.0.0.1:27501`, so only the proxy reaches them (stealth-master uses 8089 and 8090; the two
can share a machine).

Game servers report to `https://lt-master.playspoon.com` unless told otherwise: the dedicated
server with `--master <url>` or `--private`.
