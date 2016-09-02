extern crate schannel;

use std::io::{self, Read, Write};
use std::mem;

use self::schannel::tls_stream::{self, HandshakeError};
use self::schannel::tls_stream::MidHandshakeTlsStream;
use self::schannel::schannel_cred::{self, Direction};
use futures::{Async, Poll, Future};

pub struct ServerContext {
    cred: schannel_cred::Builder,
    stream: tls_stream::Builder,
}

pub struct ClientContext {
    cred: schannel_cred::Builder,
    stream: tls_stream::Builder,
}

impl ServerContext {
    pub fn handshake<S>(mut self, stream: S) -> ServerHandshake<S>
        where S: Read + Write,
    {
        let res = self.cred.acquire(Direction::Inbound);
        let res = res.map_err(HandshakeError::Failure);
        let res = res.and_then(|cred| {
            self.stream.accept(cred, stream)
        });
        ServerHandshake { inner: Handshake::new(res) }
    }
}


impl ClientContext {
    pub fn new() -> io::Result<ClientContext> {
        Ok(ClientContext {
            cred: schannel_cred::Builder::new(),
            stream: tls_stream::Builder::new(),
        })
    }

    pub fn handshake<S>(mut self,
                        domain: &str,
                        stream: S) -> ClientHandshake<S>
        where S: Read + Write,
    {
        let res = self.cred.acquire(Direction::Outbound);
        let res = res.map_err(HandshakeError::Failure);
        let res = res.and_then(|cred| {
            self.stream.domain(domain).connect(cred, stream)
        });
        ClientHandshake { inner: Handshake::new(res) }
    }
}

pub struct ServerHandshake<S> {
    inner: Handshake<S>,
}

pub struct ClientHandshake<S> {
    inner: Handshake<S>,
}

enum Handshake<S> {
    Error(io::Error),
    Stream(tls_stream::TlsStream<S>),
    Interrupted(MidHandshakeTlsStream<S>),
    Empty,
}

impl<S> Future for ClientHandshake<S>
    where S: Read + Write,
{
    type Item = TlsStream<S>;
    type Error = io::Error;

    fn poll(&mut self) -> Poll<TlsStream<S>, io::Error> {
        self.inner.poll()
    }
}

impl<S> Future for ServerHandshake<S>
    where S: Read + Write,
{
    type Item = TlsStream<S>;
    type Error = io::Error;

    fn poll(&mut self) -> Poll<TlsStream<S>, io::Error> {
        self.inner.poll()
    }
}

impl<S> Handshake<S> {
    fn new(res: Result<tls_stream::TlsStream<S>, HandshakeError<S>>)
           -> Handshake<S> {
        match res {
            Ok(s) => Handshake::Stream(s),
            Err(HandshakeError::Failure(e)) => Handshake::Error(e),
            Err(HandshakeError::Interrupted(s)) => Handshake::Interrupted(s),
        }
    }
}

impl<S> Future for Handshake<S>
    where S: Read + Write,
{
    type Item = TlsStream<S>;
    type Error = io::Error;

    fn poll(&mut self) -> Poll<TlsStream<S>, io::Error> {
        let stream = match mem::replace(self, Handshake::Empty) {
            Handshake::Error(e) => return Err(e),
            Handshake::Empty => panic!("can't poll handshake twice"),
            Handshake::Stream(s) => return Ok(TlsStream::new(s).into()),
            Handshake::Interrupted(s) => s,
        };

        // TODO: dedup with Handshake::new
        match stream.handshake() {
            Ok(s) => Ok(TlsStream::new(s).into()),
            Err(HandshakeError::Failure(e)) => Err(e),
            Err(HandshakeError::Interrupted(s)) => {
                *self = Handshake::Interrupted(s);
                Ok(Async::NotReady)
            }
        }
    }
}

pub struct TlsStream<S> {
    inner: tls_stream::TlsStream<S>,
}

impl<S> TlsStream<S> {
    fn new(s: tls_stream::TlsStream<S>) -> TlsStream<S> {
        TlsStream { inner: s }
    }
}

impl<S: Read + Write> Read for TlsStream<S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl<S: Read + Write> Write for TlsStream<S> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Extension trait for servers backed by SChannel.
pub trait ServerContextExt: Sized {
    /// Creates a new server which is ready for configuration via
    /// `schannel_cred` and `tls_stream`.
    ///
    /// Note that accepting connections will likely not work unless a public and
    /// private key are configured via the `schannel_cred` method.
    fn new() -> Self;

    /// Gets a mutable reference to the underlying SChannel credential builder,
    /// allowing further configuration.
    ///
    /// The builder here will eventually get used to initiate the client
    /// connection, and it will otherwise be configured to validate the hostname
    /// given to `handshake` by default.
    fn schannel_cred(&mut self) -> &mut schannel_cred::Builder;

    /// Gets a mutable reference to the underlying TLS stream builder, allowing
    /// further configuration.
    ///
    /// The builder here will eventually get used to initiate the client
    /// connection, and it will otherwise be configured to validate the hostname
    /// given to `handshake` by default.
    fn tls_stream(&mut self) -> &mut tls_stream::Builder;
}

impl ServerContextExt for ::ServerContext {
    fn new() -> ::ServerContext {
        ::ServerContext {
            inner: ServerContext {
                cred: schannel_cred::Builder::new(),
                stream: tls_stream::Builder::new(),
            },
        }
    }

    fn schannel_cred(&mut self) -> &mut schannel_cred::Builder {
        &mut self.inner.cred
    }

    fn tls_stream(&mut self) -> &mut tls_stream::Builder {
        &mut self.inner.stream
    }
}

/// Extension trait for clients backed by SChannel.
pub trait ClientContextExt {
    /// Gets a mutable reference to the underlying SChannel credential builder,
    /// allowing further configuration.
    ///
    /// The builder here will eventually get used to initiate the client
    /// connection, and it will otherwise be configured to validate the hostname
    /// given to `handshake` by default.
    fn schannel_cred(&mut self) -> &mut schannel_cred::Builder;

    /// Gets a mutable reference to the underlying TLS stream builder, allowing
    /// further configuration.
    ///
    /// The builder here will eventually get used to initiate the client
    /// connection, and it will otherwise be configured to validate the hostname
    /// given to `handshake` by default.
    fn tls_stream(&mut self) -> &mut tls_stream::Builder;
}

impl ClientContextExt for ::ClientContext {
    fn schannel_cred(&mut self) -> &mut schannel_cred::Builder {
        &mut self.inner.cred
    }

    fn tls_stream(&mut self) -> &mut tls_stream::Builder {
        &mut self.inner.stream
    }
}

/// Extension trait for streams backed by SChannel.
pub trait TlsStreamExt {
}

impl<S> TlsStreamExt for ::TlsStream<S> {
}
