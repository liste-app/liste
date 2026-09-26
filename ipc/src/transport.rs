//! The transport: a listener and a stream over `interprocess` local
//! sockets, plus framed send and receive.

use std::io;

use interprocess::local_socket::{
    GenericFilePath, GenericNamespaced, ListenerOptions, ToFsName, ToNsName, prelude::*,
};

use crate::paths::Endpoint;
use crate::protocol::{Message, read_frame, write_frame};

pub use interprocess::local_socket::traits::ListenerExt;
pub use interprocess::local_socket::{Listener, Stream};

fn name(endpoint: &Endpoint) -> io::Result<interprocess::local_socket::Name<'static>> {
    match endpoint {
        Endpoint::Path(p) => p.clone().to_fs_name::<GenericFilePath>(),
        Endpoint::Name(n) => n.clone().to_ns_name::<GenericNamespaced>(),
    }
}

/// Create the listener. On Unix the socket's directory is made 0700 before
/// the socket appears and the socket itself is set to 0600 right after; a
/// stale socket file from a crashed host is replaced.
pub fn bind(endpoint: &Endpoint) -> io::Result<Listener> {
    if let Some(dir) = endpoint.parent_dir() {
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    let options = ListenerOptions::new()
        .name(name(endpoint)?)
        .reclaim_name(true);
    let listener = match options.create_sync() {
        Err(e) if e.kind() == io::ErrorKind::AddrInUse => {
            // A corpse socket from a crashed host: nobody answers on it, so
            // it is safe to replace. A live host holds the store lock, which
            // is checked before we get here.
            if let Endpoint::Path(p) = endpoint {
                std::fs::remove_file(p)?;
            }
            ListenerOptions::new()
                .name(name(endpoint)?)
                .reclaim_name(true)
                .create_sync()?
        }
        other => other?,
    };
    #[cfg(unix)]
    if let Endpoint::Path(p) = endpoint {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(listener)
}

/// Connect to a listening host. Fails at once if nothing listens.
pub fn connect(endpoint: &Endpoint) -> io::Result<Stream> {
    Stream::connect(name(endpoint)?)
}

/// Send one message.
pub fn send(stream: &mut Stream, message: &Message) -> io::Result<()> {
    write_frame(stream, message)
}

/// Receive one message; `None` when the peer closed the connection.
pub fn receive(stream: &mut Stream) -> io::Result<Option<Message>> {
    read_frame(stream)
}
