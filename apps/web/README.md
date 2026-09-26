# apps/web

The web app: SvelteKit, Tailwind CSS, and shadcn-svelte. Marketing, docs, and pricing pages are prerendered; the app itself runs in SPA mode. The Rust core, including SQLite and crypto, runs as WASM inside a dedicated Web Worker. Installable PWA with a service worker for full offline use. Phase 1.

Two adapters from one source: `@sveltejs/adapter-cloudflare` for Liste Cloud and `@sveltejs/adapter-node` for the self-host image. Nothing in the app code is adapter-specific.

The SvelteKit project has not been created yet. When it is, use pnpm, and pin the adapters at or above the versions verified in `docs/ARCHITECTURE.md`'s dependency notes.
