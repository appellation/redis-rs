use std::ops::{Deref, DerefMut};

use futures::{SinkExt, TryStreamExt};
use resp::Data;
use tokio::net::{TcpStream, ToSocketAddrs};
use tokio_util::codec::{Decoder, Framed};

use crate::{codec::Codec, error::Result};

/// A TCP connection to a Redis server.
///
/// To enter PubSub mode, send the appropriate subscription command using [Self::send_cmd()] and
/// then consume the stream.
#[derive(Debug)]
pub struct Connection {
	framed: Framed<TcpStream, Codec>,
}

impl Connection {
	/// Connect to the Redis server using the provided `addr`.
	pub async fn new(addr: impl ToSocketAddrs) -> Result<Self, std::io::Error> {
		let stream = TcpStream::connect(addr).await?;
		let framed = Codec.framed(stream);
		Ok(Self { framed })
	}

	/// Send a command to the server, awaiting a single response.
	pub async fn cmd<'a, C, I>(&mut self, cmd: C) -> Result<Data<'static>>
	where
		C: IntoIterator<Item = &'a I>,
		I: 'a + AsRef<[u8]> + ?Sized,
	{
		self.send_cmd(cmd).await?;
		self.try_next().await.transpose().unwrap()
	}

	/// Send a command without waiting for a response.
	pub async fn send_cmd<'a, C, I>(&mut self, cmd: C) -> Result<()>
	where
		C: IntoIterator<Item = &'a I>,
		I: 'a + AsRef<[u8]> + ?Sized,
	{
		let data = Data::Array(Some(
			cmd.into_iter()
				.map(|bytes| Data::BulkString(Some(bytes.as_ref().into())))
				.collect(),
		));

		self.send(data).await
	}
}

impl Deref for Connection {
	// TODO: make this opaque once RFC 2515 is stable (https://github.com/rust-lang/rust/issues/63063)
	type Target = Framed<TcpStream, Codec>;

	fn deref(&self) -> &Self::Target {
		&self.framed
	}
}

impl DerefMut for Connection {
	fn deref_mut(&mut self) -> &mut Self::Target {
		&mut self.framed
	}
}

#[cfg(test)]
mod test {
	use std::env;

	use resp::Data;

	use super::Connection;

	fn redis_url() -> String {
		env::var("REDIS_URL").unwrap_or_else(|_| "localhost:6379".to_string())
	}

	#[tokio::test]
	async fn ping() {
		let mut conn = Connection::new(redis_url()).await.expect("new connection");

		let res = conn.cmd(["PING"]).await.expect("send command");
		assert_eq!(res, Data::SimpleString("PONG".into()));
	}

	#[tokio::test]
	async fn multi_ping() {
		let mut conn = Connection::new(redis_url()).await.expect("new connection");

		let res = conn.cmd(["PING"]).await.expect("send command");
		assert_eq!(res, Data::SimpleString("PONG".into()));

		let res = conn.cmd(["PING", "foobar"]).await.expect("send command");
		assert_eq!(res, Data::BulkString(Some(b"foobar"[..].into())));
	}

	#[tokio::test]
	async fn stream() {
		let mut conn = Connection::new(redis_url()).await.expect("new connection");

		// return value is ID which is dynamic
		let res_id = conn
			.cmd(["XADD", "foo", "*", "foo", "bar"])
			.await
			.expect("send command");

		let res = conn
			.cmd(["XREAD", "STREAMS", "foo", "0-0"])
			.await
			.expect("send command");

		conn.cmd(["DEL", "foo"]).await.expect("delete stream key");

		let expected = Data::Array(Some(vec![Data::Array(Some(vec![
			Data::BulkString(Some(b"foo"[..].into())),
			Data::Array(Some(vec![Data::Array(Some(vec![
				res_id,
				Data::Array(Some(vec![
					Data::BulkString(Some(b"foo"[..].into())),
					Data::BulkString(Some(b"bar"[..].into())),
				])),
			]))])),
		]))]));

		assert_eq!(res, expected);
	}
}
