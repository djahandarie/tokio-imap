use std::io;
use std::net::ToSocketAddrs;

use futures::{Async, Future, Poll, Sink};
use futures::stream::Stream;
use futures::sink::Send;
use native_tls::TlsConnector;
use tokio_core::net::{TcpStream, TcpStreamNew};
use tokio_io::AsyncRead;
use tokio_core::reactor::Handle;
use tokio_tls::{ConnectAsync, TlsConnectorExt};

use proto;

pub struct ClientState {
    state: proto::State,
    server_greeting: String,
}

pub struct Client {
    transport: proto::ImapTransport,
    state: ClientState,
}

pub enum ConnectFuture {
    #[doc(hidden)]
    TcpConnecting(TcpStreamNew, String),
    #[doc(hidden)]
    TlsHandshake(ConnectAsync<TcpStream>),
    #[doc(hidden)]
    ServerGreeting(Option<proto::ImapTransport>),
}

impl Future for ConnectFuture {
    type Item = Client;
    type Error = io::Error;
    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let mut new = None;
        if let ConnectFuture::TcpConnecting(ref mut future, ref domain) = *self {
            let stream = try_ready!(future.poll());
            let ctx = TlsConnector::builder().unwrap().build().unwrap();
            let future = ctx.connect_async(&domain, stream);
            new = Some(ConnectFuture::TlsHandshake(future));
        }
        if new.is_some() {
            *self = new.take().unwrap();
        }
        if let ConnectFuture::TlsHandshake(ref mut future) = *self {
            let transport = try_ready!(future.map_err(|e| {
                io::Error::new(io::ErrorKind::Other, e)
            }).poll()).framed(proto::ImapCodec);
            new = Some(ConnectFuture::ServerGreeting(Some(transport)));
        }
        if new.is_some() {
            *self = new.take().unwrap();
        }
        if let ConnectFuture::ServerGreeting(ref mut wrapped) = *self {
            let msg = try_ready!(wrapped.as_mut().unwrap().poll()).unwrap();
            return Ok(Async::Ready(Client {
                transport: wrapped.take().unwrap(),
                state: ClientState {
                    state: proto::State::NotAuthenticated,
                    server_greeting: msg,
                },
            }));
        }
        Ok(Async::NotReady)
    }
}

pub struct LoginFuture {
    future: Send<proto::ImapTransport>,
    clst: Option<ClientState>,
}

impl Future for LoginFuture {
    type Item = Client;
    type Error = io::Error;
    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let transport = try_ready!(self.future.poll());
        let mut state = self.clst.take().unwrap();
        state.state = proto::State::Authenticated;
        return Ok(Async::Ready(Client {
            transport: transport,
            state: state,
        }));
    }
}

impl Client {
    pub fn connect(server: &str, handle: &Handle) -> ConnectFuture {
        let addr = format!("{}:993", server);
        let addr = addr.to_socket_addrs().unwrap().next().unwrap();
        let stream = TcpStream::connect(&addr, handle);
        ConnectFuture::TcpConnecting(stream, server.to_string())
    }

    pub fn login(self, account: &str, password: &str) -> LoginFuture {
        let Client { transport, state } = self;
        let msg = format!("a001 LOGIN {} {}", account, password);
        LoginFuture {
            future: transport.send(msg),
            clst: Some(state),
        }
    }

    pub fn server_greeting(&self) -> &str {
        &self.state.server_greeting
    }
}