# Self-hosting Liste

**Liste is in early development and is not usable yet. Do not self-host it in production.** These files exist so that self-hosting is built alongside Liste Cloud from the start; they will not work until the first release.

When they do, self-hosting is intended to be a 15-minute job for someone who has run Docker before:

1. Copy `.env.example` to `.env` and fill in the values.
2. `docker compose up -d`.
3. Put a reverse proxy with TLS in front (Coolify, Caddy, Traefik, or a tunnel).
4. In any Liste app, choose "Use a self-hosted server" on the sign-in screen and enter your URL.

## What you get

Everything. All features, all clients, no license keys, no feature flags gated on payment. The only Cloud-only items are the short list in Section 18 of `docs/ARCHITECTURE.md`: billing, Sign in with Apple and Google, native-app passkeys, the push relay's server side, and inbound channels on Liste's own domains. Against a self-hosted server the native apps sign in with an email one-time code, which is a first-class login.

## Images

- `ghcr.io/liste-app/liste-server`: the sync server, the exact image Liste Cloud runs.
- `ghcr.io/liste-app/liste-web`: the web app shell (SvelteKit `adapter-node`).

Both are built by the tagged release pipeline, multi-arch (amd64, arm64). Neither is published yet.

## Migrations

The server runs pending migrations on start when `LISTE_AUTO_MIGRATE=true`, which is the compose default. It refuses to start against a database newer than itself.

## Backups

Everything the server stores is ciphertext, but a lost database with no backup is unrecoverable. Run a nightly `pg_dump`:

```
0 3 * * * docker compose exec -T postgres pg_dump -U liste liste | gzip > /backups/liste-$(date +\%F).sql.gz
```

Attachments, when they ship, live outside Postgres in S3-compatible storage.

## Push notifications

Store-signed apps can only receive push from Liste's own credentials, so a self-hosted server cannot push to them directly. Liste Cloud runs a free, optional push relay: set `LISTE_PUSH_RELAY_URL` and register once. The relay sees a device token and ciphertext, nothing else. Without it, clients fall back to background polling.

## Upgrades

Pull the new tag and restart. Release notes list any configuration changes.

## Coolify

`coolify/` holds a one-click template: the same compose file with sane defaults.
