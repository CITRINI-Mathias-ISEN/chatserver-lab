use async_std::net::UdpSocket;
use async_std::sync::RwLock;
use async_std::task;
use chatproto::core::MessageServer;
#[cfg(feature = "federation")]
use chatproto::messages::ServerReply;
use chatproto::messages::{ClientError, ClientQuery, Sequence, ServerId};
use chatproto::netproto::{decode, encode};
use std::io::Cursor;
use std::net::IpAddr;
use std::sync::Arc;
use structopt::StructOpt;

#[derive(StructOpt)]
struct Opt {
  #[structopt(long, default_value = "4666")]
  /// port to listen for clients on
  cport: u16,

  #[structopt(long, default_value = "0.0.0.0")]
  /// address to listen for clients on
  clisten: IpAddr,

  #[structopt(long, default_value = "4667")]
  /// port to listen for servers on
  sport: u16,

  #[structopt(long, default_value = "0.0.0.0")]
  /// address to listen for servers on
  slisten: IpAddr,
}

#[cfg(feature = "federation")]
async fn server_thread<S: MessageServer>(
  listen: IpAddr,
  port: u16,
  srv: &RwLock<S>,
) -> std::io::Result<()> {
  let socket = UdpSocket::bind((listen, port)).await?;
  log::info!("Listening for servers on {}", socket.local_addr()?);
  let mut buf = vec![0u8; 8192];
  loop {
    let (n, peer) = socket.recv_from(&mut buf).await?;
    let mut cursor = Cursor::new(buf[..n].to_vec());
    match decode::server(&mut cursor) {
      Err(rr) => log::error!("Could not decode server message from {}: {}", peer, rr),
      Ok(msg) => match srv.write().await.handle_server_message(msg).await {
        ServerReply::Outgoing(_) => todo!(),
        ServerReply::EmptyRoute => todo!(),
        ServerReply::Error(rr) => {
          log::error!("Error occured when handling message from {}: {}", peer, rr)
        }
      },
    }
  }
}

async fn handle_client_query<S: MessageServer>(
  srv: &RwLock<S>,
  m: Sequence<ClientQuery>,
) -> anyhow::Result<Vec<u8>> {
  log::debug!("received {:?}", m);
  let src = m.src;

  let lock = srv.write().await;

  // handle register
  if let ClientQuery::Register(name) = &m.content {
    log::debug!("handle register message");
    let name = name.clone();
    match lock.handle_sequenced_message(m).await {
      Ok(_) => (),
      Err(ClientError::UnknownClient) => (),
      Err(rr) => {
        anyhow::bail!("Error when handling register message: {}", rr);
      }
    }
    let id = lock.register_local_client(name).await;
    let mut ocurs = Cursor::new(Vec::new());
    encode::clientid(&mut ocurs, &id)?;
    return Ok(ocurs.into_inner());
  }

  match lock.handle_sequenced_message(m).await? {
    ClientQuery::Poll => {
      let repl = lock.client_poll(src).await;
      log::debug!(" -> poll {:?}", repl);
      let mut ocurs = Cursor::new(Vec::new());
      encode::client_poll_reply(&mut ocurs, &repl)?;
      Ok(ocurs.into_inner())
    }
    ClientQuery::ListUsers => {
      let repl = lock.list_users().await;
      let mut ocurs = Cursor::new(Vec::new());
      encode::userlist(&mut ocurs, &repl)?;
      Ok(ocurs.into_inner())
    }
    ClientQuery::Register(_) => {
      anyhow::bail!("Unexpected register message from enrolled client")
    }
    ClientQuery::Message(msg) => {
      let repl = lock.handle_client_message(src, msg).await;
      let mut ocurs = Cursor::new(Vec::new());
      encode::client_replies(&mut ocurs, &repl)?;
      Ok(ocurs.into_inner())
    }
  }
}

async fn client_thread<S: MessageServer>(
  listen: IpAddr,
  port: u16,
  srv: &RwLock<S>,
) -> anyhow::Result<()> {
  let socket = UdpSocket::bind((listen, port)).await?;
  log::info!("Listening for clients on {}", socket.local_addr()?);
  let mut buf = vec![0u8; 8192];
  loop {
    let (n, peer) = socket.recv_from(&mut buf).await?;
    let mut cursor = Cursor::new(buf[..n].to_vec());
    match decode::sequence(&mut cursor, decode::client_query) {
      Err(rr) => log::error!("Could not decode message from {}: {}", peer, rr),
      Ok(m) => match handle_client_query(srv, m).await {
        Ok(msg) => {
          log::debug!("sending message {:?}", msg);
          match socket.send_to(&msg, peer).await {
            Ok(_) => (),
            Err(rr) => log::error!("Error when sending message to {}: {}", peer, rr),
          }
        }
        Err(rr) => log::error!("Error when handling message to {}: {}", peer, rr),
      },
    }
  }
}

fn main() {
  pretty_env_logger::init();
  let opt = Opt::from_args();

  let server = chatproto::solutions::citrini_repplinger_vassoille::Server::new(ServerId::default());
  let clock = Arc::new(RwLock::new(server));
  #[cfg(feature = "federation")]
  let slock = clock.clone();

  task::block_on(async move {
    let cchild = task::spawn(async move {
      if let Err(rr) = client_thread(opt.clisten, opt.cport, &clock).await {
        log::error!("{}", rr)
      }
    });
    #[cfg(feature = "federation")]
    let schild = task::spawn(async move {
      if let Err(rr) = server_thread(opt.slisten, opt.sport, &slock).await {
        log::error!("{}", rr)
      }
    });
    cchild.await;
    #[cfg(feature = "federation")]
    let _ = schild.cancel().await;
  });
}
