//! The desktop host (Section 3): the one process per device that owns the
//! store, holds the keys, serves local IPC, and runs sync.
//!
//! A [`Host`] takes the store lock, opens SQLite, loads or creates the
//! device identity, and only then creates its IPC endpoint (the readiness
//! rule: the socket exists once the store is open, locked or not). Each
//! connection is served on its own thread; every request goes through the
//! same store methods the GUI calls in-process, so an IPC client can do
//! nothing the app cannot. A maintenance thread folds the write-ahead log
//! and runs the [`SyncRunner`] whenever ops are pending. Dropping the host,
//! or calling [`Host::shutdown`], closes the endpoint and releases the lock.
//!
//! Interpretations: a device that has never received account keys is a
//! local-only store and starts unlocked; once a keyring exists, the store
//! is locked until the keyring is unlocked. The host's time zone is the
//! system's; a client may name another zone per capture.

mod handlers;
pub mod lock;
pub mod scenario;
pub mod sync;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use jiff::tz::TimeZone;
use liste_ipc::Endpoint;
use liste_ipc::protocol::{Body, IpcError, Message, PROTOCOL_VERSION, Request, Response};
use liste_ipc::transport::{self, ListenerExt, Stream};
use serde::{Deserialize, Serialize};

use crate::crypto::{Keyring, MemoryKeyStore};
use crate::ids::Id;
use crate::parse::Locale;
use crate::store::{Store, StoreError};

pub use lock::StoreLock;
pub use sync::{NoopSync, SyncError, SyncReport, SyncRunner};

/// The version the host reports in the handshake.
pub const HOST_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error("the store at {0} is held by another Liste process")]
    StoreHeld(PathBuf),
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("cannot listen on {0}: {1}")]
    Listen(Endpoint, std::io::Error),
}

/// How to start a host.
pub struct HostConfig {
    pub data_dir: PathBuf,
    pub endpoint: Endpoint,
    pub locale: Locale,
    /// Start with the store locked even without a keyring, for tests.
    pub locked: bool,
    pub sync: Box<dyn SyncRunner>,
    pub keyring: Keyring,
    pub maintenance_interval: Duration,
}

impl HostConfig {
    /// Defaults: US locale, unlocked, no-op sync, in-memory keystore,
    /// maintenance every two seconds.
    pub fn new(data_dir: impl Into<PathBuf>, endpoint: Endpoint) -> HostConfig {
        HostConfig {
            data_dir: data_dir.into(),
            endpoint,
            locale: Locale::US,
            locked: false,
            sync: Box::new(NoopSync),
            keyring: Keyring::new(Box::new(MemoryKeyStore::new())),
            maintenance_interval: Duration::from_secs(2),
        }
    }
}

/// The device identity kept beside the database. Not secret.
#[derive(Serialize, Deserialize)]
struct Identity {
    device_id: Id,
    space_id: Id,
}

fn load_identity(dir: &Path) -> Result<Identity, HostError> {
    let path = dir.join("device.json");
    if let Ok(bytes) = std::fs::read(&path)
        && let Ok(identity) = serde_json::from_slice::<Identity>(&bytes)
    {
        return Ok(identity);
    }
    let identity = Identity {
        device_id: Id::new(),
        space_id: Id::new(),
    };
    let bytes = serde_json::to_vec_pretty(&identity).expect("identity serializes");
    std::fs::write(&path, bytes).map_err(|e| HostError::Io(path, e))?;
    Ok(identity)
}

pub(crate) struct Inner {
    pub(crate) store: Mutex<Store>,
    pub(crate) keyring: Mutex<Keyring>,
    pub(crate) sync: Mutex<Box<dyn SyncRunner>>,
    pub(crate) data_dir: PathBuf,
    pub(crate) endpoint: Endpoint,
    pub(crate) locale: Locale,
    pub(crate) device_id: Id,
    pub(crate) space_id: Id,
    pub(crate) tz: TimeZone,
    shutdown: AtomicBool,
    _lock: StoreLock,
}

/// A running host. Dropping it shuts it down.
pub struct Host {
    inner: Arc<Inner>,
    threads: Vec<JoinHandle<()>>,
    listener_thread: Option<JoinHandle<()>>,
    stopped: bool,
}

impl std::fmt::Debug for Host {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Host({})", self.inner.endpoint)
    }
}

impl Host {
    /// Take the store lock, open the store, then listen.
    pub fn start(config: HostConfig) -> Result<Host, HostError> {
        let lock = StoreLock::acquire(&config.data_dir)?;
        let identity = load_identity(&config.data_dir)?;
        let mut store = Store::open(config.data_dir.join("liste.db"), identity.device_id)?;
        let mut keyring = config.keyring;
        let has_keys = keyring.is_initialized().unwrap_or(false);
        if has_keys && !keyring.is_unlocked() {
            let _ = keyring.unlock();
        }
        if config.locked || (has_keys && !keyring.is_unlocked()) {
            store.lock();
        }
        let tz = TimeZone::try_system().unwrap_or(TimeZone::UTC);
        let inner = Arc::new(Inner {
            store: Mutex::new(store),
            keyring: Mutex::new(keyring),
            sync: Mutex::new(config.sync),
            data_dir: config.data_dir,
            endpoint: config.endpoint.clone(),
            locale: config.locale,
            device_id: identity.device_id,
            space_id: identity.space_id,
            tz,
            shutdown: AtomicBool::new(false),
            _lock: lock,
        });
        // The store is open: now, and only now, the endpoint appears.
        let listener = transport::bind(&config.endpoint)
            .map_err(|e| HostError::Listen(config.endpoint.clone(), e))?;
        let accept_inner = inner.clone();
        let listener_thread = std::thread::Builder::new()
            .name("liste-host-accept".into())
            .spawn(move || accept_loop(accept_inner, listener))
            .map_err(|e| HostError::Io(PathBuf::from("thread"), e))?;
        let maint_inner = inner.clone();
        let interval = config.maintenance_interval;
        let maint = std::thread::Builder::new()
            .name("liste-host-maintenance".into())
            .spawn(move || maintenance_loop(maint_inner, interval))
            .map_err(|e| HostError::Io(PathBuf::from("thread"), e))?;
        Ok(Host {
            inner,
            threads: vec![maint],
            listener_thread: Some(listener_thread),
            stopped: false,
        })
    }

    pub fn endpoint(&self) -> &Endpoint {
        &self.inner.endpoint
    }

    pub fn data_dir(&self) -> &Path {
        &self.inner.data_dir
    }

    pub fn device_id(&self) -> Id {
        self.inner.device_id
    }

    pub fn space_id(&self) -> Id {
        self.inner.space_id
    }

    pub fn is_locked(&self) -> bool {
        self.inner
            .store
            .lock()
            .map(|s| s.is_locked())
            .unwrap_or(true)
    }

    /// Lock the store: data requests answer `locked` until unlocked.
    pub fn lock(&self) {
        if let Ok(mut s) = self.inner.store.lock() {
            s.lock();
        }
        if let Ok(mut k) = self.inner.keyring.lock() {
            k.lock();
        }
    }

    /// Unlock the store for data requests. The GUI calls this after the
    /// keyring is unlocked; a CLI or MCP client never can.
    pub fn unlock(&self) {
        if let Ok(mut s) = self.inner.store.lock() {
            s.unlock();
        }
    }

    /// In-process access to the store, as the GUI has.
    pub fn with_store<R>(&self, f: impl FnOnce(&mut Store) -> R) -> R {
        let mut store = self.inner.store.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut store)
    }

    /// In-process access to the sync runner, for tests and status.
    pub fn with_sync<R>(&self, f: impl FnOnce(&mut dyn SyncRunner) -> R) -> R {
        let mut sync = self.inner.sync.lock().unwrap_or_else(|e| e.into_inner());
        f(sync.as_mut())
    }

    /// Run the sync runner once, now.
    pub fn sync_now(&self) -> Result<SyncReport, SyncError> {
        sync_once(&self.inner)
    }

    /// Serve a request in-process, exactly as an IPC client would have it
    /// served.
    pub fn handle(&self, request: Request) -> Response {
        handlers::handle(&self.inner, request)
    }

    /// Close the endpoint, stop the threads, release the lock.
    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        if self.stopped {
            return;
        }
        self.stopped = true;
        self.inner.shutdown.store(true, Ordering::SeqCst);
        // Wake the accept loop with a connection to ourselves.
        let _ = transport::connect(&self.inner.endpoint);
        if let Some(t) = self.listener_thread.take() {
            let _ = t.join();
        }
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
        if let Endpoint::Path(p) = &self.inner.endpoint {
            let _ = std::fs::remove_file(p);
        }
        if let Ok(store) = self.inner.store.lock() {
            let _ = store.checkpoint();
        }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.stop();
    }
}

fn accept_loop(inner: Arc<Inner>, listener: transport::Listener) {
    for conn in listener.incoming() {
        if inner.shutdown.load(Ordering::SeqCst) {
            break;
        }
        let Ok(stream) = conn else { continue };
        let inner = inner.clone();
        let _ = std::thread::Builder::new()
            .name("liste-host-client".into())
            .spawn(move || serve(inner, stream));
    }
}

fn serve(inner: Arc<Inner>, mut stream: Stream) {
    // Handshake first.
    let Ok(Some(first)) = transport::receive(&mut stream) else {
        return;
    };
    let reply = match first.body {
        Body::Request(Request::Hello {
            protocol_version, ..
        }) if protocol_version == PROTOCOL_VERSION => Response::Hello {
            protocol_version: PROTOCOL_VERSION,
            host_version: HOST_VERSION.to_owned(),
        },
        Body::Request(Request::Hello {
            protocol_version, ..
        }) => Response::Error(IpcError::VersionMismatch {
            host: PROTOCOL_VERSION,
            client: protocol_version,
        }),
        _ => Response::Error(IpcError::Invalid {
            message: "the first message must be hello".into(),
        }),
    };
    let ok = matches!(reply, Response::Hello { .. });
    if transport::send(
        &mut stream,
        &Message {
            id: first.id,
            body: Body::Response(reply),
        },
    )
    .is_err()
        || !ok
    {
        return;
    }
    while let Ok(Some(message)) = transport::receive(&mut stream) {
        if inner.shutdown.load(Ordering::SeqCst) {
            break;
        }
        let response = match message.body {
            Body::Request(request) => handlers::handle(&inner, request),
            Body::Response(_) => Response::Error(IpcError::Invalid {
                message: "clients send requests".into(),
            }),
        };
        if transport::send(
            &mut stream,
            &Message {
                id: message.id,
                body: Body::Response(response),
            },
        )
        .is_err()
        {
            break;
        }
    }
}

fn maintenance_loop(inner: Arc<Inner>, interval: Duration) {
    let tick = Duration::from_millis(50);
    let mut elapsed = Duration::ZERO;
    while !inner.shutdown.load(Ordering::SeqCst) {
        std::thread::sleep(tick);
        elapsed += tick;
        if elapsed < interval {
            continue;
        }
        elapsed = Duration::ZERO;
        if let Ok(store) = inner.store.lock() {
            let _ = store.checkpoint();
        }
        let pending = inner
            .store
            .lock()
            .ok()
            .and_then(|s| s.pending_count(inner.space_id).ok())
            .unwrap_or(0);
        if pending > 0 {
            let _ = sync_once(&inner);
        }
    }
}

pub(crate) fn sync_once(inner: &Inner) -> Result<SyncReport, SyncError> {
    let mut store = inner.store.lock().unwrap_or_else(|e| e.into_inner());
    let keyring = inner.keyring.lock().unwrap_or_else(|e| e.into_inner());
    let mut sync = inner.sync.lock().unwrap_or_else(|e| e.into_inner());
    sync.sync(&mut store, &keyring)
}
