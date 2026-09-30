---
title: 'TLS and SSH tunnels'
description: 'Encrypt connections with TLS, verify certificates, and reach private databases through an SSH bastion.'
---

# TLS and SSH tunnels

## TLS

TLS is built into quarry (it uses rustls), so there's nothing to install. Set the mode with
`--ssl-mode` or `sslmode=` in the URL:

```sh
quarry "postgres://me@db.example.com/app?sslmode=verify-full"
quarry mysql://me@db.example.com/shop --ssl-mode require
```

| Mode | Encrypts | Checks the certificate |
|---|---|---|
| `disable` | No | – |
| `prefer` (default) | If the server supports it | No |
| `require` | Always | No |
| `verify-ca` | Always | The certificate is signed by a trusted CA, but the host name isn't checked |
| `verify-full` | Always | The CA **and** that the certificate matches the host name |

`prefer` and `require` protect against eavesdropping but not against someone impersonating the
server. For anything that crosses a network you don't control, use `verify-full`.

Other spellings are accepted too: `off`, `on`, `true`, `false`, `required`, `preferred` and
`verify-identity`.

### Certificates

| Flag | URL parameter | Meaning |
|---|---|---|
| `--ssl-ca FILE` | `sslrootcert=FILE` | CA certificate(s) to trust, in PEM. Without it, quarry trusts the usual public CAs and your system's certificates. |
| `--ssl-cert FILE` | `sslcert=FILE` | Client certificate (PEM) |
| `--ssl-key FILE` | `sslkey=FILE` | Client private key (PEM) |

A client certificate is only used when both `--ssl-cert` and `--ssl-key` are given.

### Per-database notes

- **PostgreSQL:** connections over a Unix socket never use TLS.
- **MySQL:** with `prefer`, quarry falls back to an unencrypted connection if the TLS handshake fails.
  A socket connection with `prefer` is unencrypted.
- **SQLite:** a local file, so TLS doesn't apply.

The TUI status bar shows `TLS` when the connection is encrypted, and `\conninfo` reports it in the
REPL.

## SSH tunnels

To reach a database that's only accessible from a bastion host, let quarry open an SSH tunnel:

```sh
quarry postgres://me@10.0.0.5/app --ssh deploy@bastion.example.com
quarry mysql://root@db.internal/shop --ssh deploy@bastion.example.com:2222 --ssh-key ~/.ssh/id_ed25519
```

The format is `[user@]host[:port]`. You can also put it in a URL (`?ssh=deploy@bastion`) or on a
[saved connection](/advanced/saved-connections) (`ssh = "deploy@bastion"`).

### How it works

quarry runs your system's `ssh`:

```
ssh -N -o ExitOnForwardFailure=yes -o ServerAliveInterval=30 \
    -L 127.0.0.1:<free port>:<db host>:<db port> [-p port] [-i key] user@bastion
```

The database host and port are as seen **from the bastion**, so private addresses such as
`10.0.0.5` or `db.internal` work. quarry waits up to 15 seconds for the tunnel, connects through it,
and closes it when you disconnect.

Because it's your own `ssh`, everything in `~/.ssh/config` applies: host aliases, `ProxyJump`, agent
forwarding and keys. If `ssh bastion.example.com` works in your terminal, the tunnel will too.

The TUI shows `⇄ ssh` in the status bar while a tunnel is in use.

::: warning
Through a tunnel quarry connects to `127.0.0.1`, so `verify-full` checks the certificate against
`127.0.0.1` rather than the database's real name. Use `verify-ca`, or add `127.0.0.1` to the
certificate, if you need TLS verification over a tunnel.
:::
