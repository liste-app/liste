# infra

Deployment tooling for Liste Cloud (Section 19 of `docs/ARCHITECTURE.md`): Wrangler configuration for the web app and the container-backed sync Worker, R2 and appcast tooling for desktop downloads, and the migrations runner used by CI.

Nothing in this folder contains a secret. Runtime secrets live in Cloudflare; build-time signing material lives in GitHub Actions encrypted secrets. The sync server itself contains no Cloudflare-specific code; everything in here is the layer around it.

Self-hosting does not use anything in this folder. See `deploy/self-host/`.
