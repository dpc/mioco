use std::io;
use std::net::SocketAddr;
use mio;
use std;
use super::super::{Evented, AsyncIO};

pub use mio::tcp::Shutdown;

/// TCP Listener
pub type TcpListener = AsyncIO<mio::net::TcpListener>;

impl TcpListener {
    /// Local address
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.io.local_addr()
    }

    /// TODO: document
    pub fn take_socket_error(&self) -> io::Result<Option<io::Error>> {
        self.io.take_error()
    }

    /// Try cloning the listener descriptor.
    pub fn try_clone(&self) -> io::Result<TcpListener> {
        self.io.try_clone().map(AsyncIO::new)
    }
}

impl TcpListener {
    /// Bind to a port
    pub fn bind(addr: &SocketAddr) -> io::Result<Self> {
        mio::tcp::TcpListener::bind(addr).map(AsyncIO::new)
    }

    /// Creates a new TcpListener from an instance of a `std::net::TcpListener` type.
    pub fn from_listener(listener: std::net::TcpListener, addr: &SocketAddr) -> io::Result<Self> {
        mio::tcp::TcpListener::from_listener(listener, addr).map(AsyncIO::new)
    }

    pub fn accept(&self) -> io::Result<(TcpStream, std::net::SocketAddr)> {
        loop {
            let res = self.io.accept();

            match res {
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    self.block_on(mio::Ready::readable());
                }
                Ok((s, a)) => return Ok((AsyncIO::new(s), a)),
                Err(e) => {
                    return Err(e);
                }
            }
        }
    }
}

/// TCP Stream
pub type TcpStream = AsyncIO<mio::tcp::TcpStream>;

impl TcpStream {
    /// Create a new TCP stream an issue a non-blocking connect to the specified address.
    pub fn connect(addr: &SocketAddr) -> io::Result<Self> {
        let stream =
            mio::tcp::TcpStream::connect(addr).map(|t| {
                                                       let stream = AsyncIO::new(t);
                                                       stream.block_on(mio::Ready::writable());
                                                       stream
                                                   });

        if let Ok(ref stream) = stream {
            if let Err(err) = stream.io.take_error() {
                return Err(err);
            }
        }

        stream

    }

    /// Creates a new TcpStream from the pending socket inside the given
    /// `std::net::TcpBuilder`, connecting it to the address specified.
    pub fn connect_stream(stream: std::net::TcpStream, addr: &SocketAddr) -> io::Result<Self> {
        let stream = mio::tcp::TcpStream::connect_stream(stream, addr).map(|t| {
            let stream = AsyncIO::new(t);
            stream.block_on(mio::Ready::writable());

            stream
        });

        if let Ok(ref stream) = stream {
            if let Err(err) = stream.io.take_error() {
                return Err(err);
            }
        }

        stream
    }

    /// Local address of connection.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.io.local_addr()
    }

    /// Peer address of connection.
    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.io.peer_addr()
    }

    /// Shutdown the connection.
    pub fn shutdown(&self, how: mio::tcp::Shutdown) -> io::Result<()> {
        self.io.shutdown(how)
    }

    /// Set `no_delay`.
    pub fn set_nodelay(&self, nodelay: bool) -> io::Result<()> {
        self.io.set_nodelay(nodelay)
    }

    /// TODO: document
    pub fn take_error(&self) -> io::Result<Option<io::Error>> {
        self.io.take_error()
    }

    /// Try cloning the socket descriptor.
    pub fn try_clone(&self) -> io::Result<TcpStream> {
        self.io.try_clone().map(AsyncIO::new)
    }
}
