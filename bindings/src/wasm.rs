//! `wasm-bindgen` wrapper used by the web app. The core runs inside a
//! dedicated Web Worker; the UI talks to it through a thin message-passing
//! layer so the main thread stays free (Section 11).
