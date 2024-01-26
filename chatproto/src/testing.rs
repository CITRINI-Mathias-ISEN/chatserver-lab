use std::collections::HashMap;

use anyhow::Context;

use crate::{client::Client, core::*, messages::*};

async fn sequence_correct<M: MessageServer>() -> Result<(), ClientError> {
  let sid = ServerId::default();
  let server: M = MessageServer::new(sid);
  let c1 = server.register_local_client("user1".to_string()).await;
  let c2 = server.register_local_client("user2".to_string()).await;
  let mut client1 = Client::new(c1);
  let mut client2 = Client::new(c2);

  // send 1000 messages, correctly sequenced
  for i in 0..100 {
    let message = if i & 1 == 0 {
      client1.sequence(())
    } else {
      client2.sequence(())
    };
    server.handle_sequenced_message(message).await?;
  }
  Ok(())
}

async fn sequence_unknown_user<M: MessageServer>() -> anyhow::Result<()> {
  let sid = ServerId::default();
  let server: M = MessageServer::new(sid);
  let c1 = ClientId::default();
  let mut client1 = Client::new(c1);

  let message = client1.sequence(());
  match server.handle_sequenced_message(message).await {
    Err(ClientError::UnknownClient) => Ok(()),
    r => anyhow::bail!("Expected Err(UnknownClient), but got {:?}", r),
  }
}

async fn sequence_bad<M: MessageServer>() -> anyhow::Result<()> {
  let sid = ServerId::default();
  let server: M = MessageServer::new(sid);
  let c1 = server.register_local_client("user 1".to_string()).await;
  let mut client1 = Client::new(c1);
  let seq1 = client1.sequence(());
  let mut seq2 = client1.sequence(());
  seq2.seqid = seq1.seqid;
  server.handle_sequenced_message(seq1).await?;
  match server.handle_sequenced_message(seq2).await {
    Err(ClientError::SequenceError) => Ok(()),
    t => anyhow::bail!("expected a sequence error, got {:?}", t),
  }
}

async fn workproof_bad<M: MessageServer>() -> anyhow::Result<()> {
  let sid = ServerId::default();
  let server: M = MessageServer::new(sid);
  let c1 = server.register_local_client("user 1".to_string()).await;
  let r = server
      .handle_sequenced_message(Sequence {
        seqid: 1,
        src: c1,
        workproof: 0,
        content: (),
      })
      .await;
  match r {
    Err(ClientError::WorkProofError) => Ok(()),
    rr => anyhow::bail!("expected a workproof error, got {:?}", rr),
  }
}

async fn sequence_multiple_problems<M: MessageServer>() -> anyhow::Result<()> {
  let sid = ServerId::default();
  let server: M = MessageServer::new(sid);
  let c1 = ClientId::default();
  let r = server
      .handle_sequenced_message(Sequence {
        seqid: 1,
        src: c1,
        workproof: 0,
        content: (),
      })
      .await;
  match r {
    Err(ClientError::WorkProofError) => Ok(()),
    rr => anyhow::bail!("expected a workproof error, got {:?}", rr),
  }
}

async fn simple_client_test<M: MessageServer>() -> anyhow::Result<()> {
  let sid = ServerId::default();
  let server: M = MessageServer::new(sid);

  let c1 = server.register_local_client("user 1".to_string()).await;
  let c2 = server.register_local_client("user 2".to_string()).await;
  let r = server
      .handle_client_message(
        c1,
        ClientMessage::Text {
          dest: c2,
          content: "hello".into(),
        },
      )
      .await;
  if r != &[ClientReply::Delivered] {
    anyhow::bail!("expected a single delivered message, got {:?}", r)
  }
  let reply = server.client_poll(c2).await;
  let expected = ClientPollReply::Message {
    src: c1,
    content: "hello".into(),
  };
  if reply != expected {
    anyhow::bail!(
      "Did not receive expected message, expected {:?}, received {:?}",
      expected,
      reply
    );
  }
  Ok(())
}

async fn list_users_test<M: MessageServer>() -> anyhow::Result<()> {
  let sid = ServerId::default();
  let server: M = MessageServer::new(sid);
  let mut usermap = HashMap::new();
  for n in 0..100_u32 {
    let username = format!("user {n}");
    let id = server.register_local_client(username.clone()).await;
    usermap.insert(id, username);
  }
  let actual = server.list_users().await;

  if actual != usermap {
    anyhow::bail!("Incorrect user map");
  }
  Ok(())
}

/// sends 100 single messages, and 100 multiple recipients messages
async fn multiple_client_messages_test<M: MessageServer>() -> anyhow::Result<()> {
  let sid = ServerId::default();
  let server: M = MessageServer::new(sid);

  let c1 = server.register_local_client("user 1".to_string()).await;
  let c2 = server.register_local_client("user 2".to_string()).await;
  let c3 = server.register_local_client("user 3".to_string()).await;
  for i in 0..100 {
    let r = server
        .handle_client_message(
          c1,
          ClientMessage::Text {
            dest: c2,
            content: i.to_string(),
          },
        )
        .await;
    if r != [ClientReply::Delivered] {
      anyhow::bail!("A> Could not deliver message {}, got {:?}", i, r);
    }
  }
  for i in 0..100 {
    let r = server
        .handle_client_message(
          c1,
          ClientMessage::MText {
            dest: vec![c2, c3],
            content: (i + 100).to_string(),
          },
        )
        .await;
    if r != [ClientReply::Delivered, ClientReply::Delivered] {
      anyhow::bail!("B> Could not deliver message {}, got {:?}", i, r);
    }
  }

  for i in 0..200 {
    let reply = server.client_poll(c2).await;
    let expected_reply = ClientPollReply::Message {
      src: c1,
      content: i.to_string(),
    };
    if reply != expected_reply {
      anyhow::bail!(
        "A> Did not receive expected message {}, received {:?}",
        i,
        reply
      );
    }
  }
  for i in 100..200 {
    let reply = server.client_poll(c3).await;
    let expected_reply = ClientPollReply::Message {
      src: c1,
      content: i.to_string(),
    };
    if reply != expected_reply {
      anyhow::bail!(
        "B> Did not receive expected message {}, received {:?}",
        i,
        reply
      );
    }
  }
  let reply = server.client_poll(c2).await;
  if reply != ClientPollReply::Nothing {
    anyhow::bail!(
      "Did not receive the expected Nothing (for client 2) reply, got {:?}",
      reply
    );
  }
  let reply = server.client_poll(c3).await;
  if reply != ClientPollReply::Nothing {
    anyhow::bail!(
      "Did not receive the expected Nothing (for client 3) reply, got {:?}",
      reply
    );
  }
  Ok(())
}

async fn mixed_results_client_message<M: MessageServer>() -> anyhow::Result<()> {
  let sid = ServerId::default();
  let server: M = MessageServer::new(sid);

  let c1 = server.register_local_client("user 1".to_string()).await;
  let c2 = server.register_local_client("user 2".to_string()).await;
  let c3 = ClientId::default();

  let m = server
      .handle_client_message(
        c1,
        ClientMessage::MText {
          dest: vec![c2, c3],
          content: "Hello".to_string(),
        },
      )
      .await;
  if m != [ClientReply::Delivered, ClientReply::Delayed] {
    anyhow::bail!("Expected Delivered/Delayed, but got {:?}", m)
  }
  Ok(())
}

async fn mailbox_full<M: MessageServer>() -> anyhow::Result<()> {
  let sid = ServerId::default();
  let server: M = MessageServer::new(sid);

  let c1 = server.register_local_client("user 1".to_string()).await;
  let c2 = server.register_local_client("user 2".to_string()).await;

  for n in 0..MAILBOX_SIZE {
    let m = server
        .handle_client_message(
          c1,
          ClientMessage::Text {
            dest: c2,
            content: format!("{n}"),
          },
        )
        .await;
    if m != [ClientReply::Delivered] {
      anyhow::bail!("Expected Delivered, but got {:?}", m)
    }
  }
  let m = server
      .handle_client_message(
        c1,
        ClientMessage::Text {
          dest: c2,
          content: "FULL".into(),
        },
      )
      .await;
  if m != [ClientReply::Error(ClientError::BoxFull(c2))] {
    anyhow::bail!("Expected BoxFull, but got {:?}", m)
  }
  Ok(())
}

#[cfg(feature = "federation")]
async fn message_to_outer_user<M: MessageServer>() -> anyhow::Result<()> {
  let sid = ServerId::default();
  let server: M = MessageServer::new(sid);

  let c1 = server.register_local_client("user 1".to_string()).await;
  let s1 = ServerId::default();
  let s2 = ServerId::default();
  let s3 = ServerId::default();
  let euuid = ClientId::default();

  log::debug!("route: {} -> {} -> {} -> us", s1, s2, s3);

  let r = server
      .handle_server_message(ServerMessage::Announce {
        route: vec![s1, s2, s3],
        clients: HashMap::from([(euuid, "external user".into())]),
      })
      .await;
  if r != ServerReply::Outgoing(Vec::new()) {
    anyhow::bail!("Expected empty outgoing answer, got {:?}", r);
  }
  assert_eq!(r, ServerReply::Outgoing(Vec::new()));
  let r = server
      .handle_client_message(
        c1,
        ClientMessage::Text {
          dest: euuid,
          content: "Hello".to_string(),
        },
      )
      .await;
  let expected = [ClientReply::Transfer(
    s3,
    ServerMessage::Message(FullyQualifiedMessage {
      src: c1,
      srcsrv: sid,
      dsts: vec![(euuid, s1)],
      content: "Hello".to_string(),
    }),
  )];

  if r != expected {
    anyhow::bail!("Expected {:?}\n   , got {:?}", expected, r)
  }

  Ok(())
}

#[cfg(feature = "federation")]
async fn message_to_outer_user_delayed<M: MessageServer>() -> anyhow::Result<()> {
  let sid = ServerId::default();
  let server: M = MessageServer::new(sid);

  let c1 = server.register_local_client("user 1".to_string()).await;
  let s1 = ServerId::default();
  let s2 = ServerId::default();
  let s3 = ServerId::default();
  let euuid = ClientId::default();

  log::debug!("route: {} -> {} -> {} -> us", s1, s2, s3);

  let r = server
      .handle_client_message(
        c1,
        ClientMessage::Text {
          dest: euuid,
          content: "Hello".to_string(),
        },
      )
      .await;
  if r != [ClientReply::Delayed] {
    anyhow::bail!("Expected a delayed message first, but got {:?}", r);
  }
  let r = server
      .handle_server_message(ServerMessage::Announce {
        route: vec![s1, s2, s3],
        clients: HashMap::from([(euuid, "external user".into())]),
      })
      .await;
  let expected = ServerReply::Outgoing(vec![Outgoing {
    nexthop: s3,
    message: FullyQualifiedMessage {
      src: c1,
      srcsrv: sid,
      dsts: vec![(euuid, s1)],
      content: "Hello".to_string(),
    },
  }]);
  if r != expected {
    anyhow::bail!("Expected {:?}\n,    got {:?}", expected, r);
  }

  Ok(())
}

#[cfg(feature = "federation")]
async fn routing_test<M: MessageServer>() -> anyhow::Result<()> {
  let sid = ServerId::default();
  let server: M = MessageServer::new(sid);

  let c1 = server.register_local_client("user 1".to_string()).await;
  /* map:

        us - s1 - s2
         |         |
        s5 - s4 - s3
   */
  let s1 = ServerId::from(1);
  let s2 = ServerId::from(2);
  let s3 = ServerId::from(3);
  let s4 = ServerId::from(4);
  let s5 = ServerId::from(5);
  let s4_user = ClientId::default();
  // first advertise the long route
  let r = server
      .handle_server_message(ServerMessage::Announce {
        route: vec![s4, s3, s2, s1],
        clients: HashMap::from([(s4_user, "s4 user".into())]),
      })
      .await;
  let expected_empty_out = ServerReply::Outgoing(Vec::new());
  if r != expected_empty_out {
    anyhow::bail!("msg1: Expected {:?}\n,    got {:?}", expected_empty_out, r);
  }
  assert_eq!(server.route_to(s4).await, Some(vec![sid, s1, s2, s3, s4]));
  println!("First routing test passed");
  // now advertise another alternative route
  let r = server
      .handle_server_message(ServerMessage::Announce {
        route: vec![s2, s3, s4, s5],
        clients: HashMap::new(),
      })
      .await;
  if r != expected_empty_out {
    anyhow::bail!("msg3: Expected {:?}\n,    got {:?}", expected_empty_out, r);
  }
  println!("Testing shortest route...");
  assert_eq!(server.route_to(s4).await, Some(vec![sid, s5, s4]));
  println!("Second routing test passed");
  Ok(())
}

async fn all_tests<M: MessageServer>(counter: &mut usize) -> anyhow::Result<()> {
  sequence_correct::<M>()
      .await
      .with_context(|| "sequence_correct")?;
  *counter = 1;
  sequence_bad::<M>().await.with_context(|| "sequence_bad")?;
  *counter += 1;
  workproof_bad::<M>()
      .await
      .with_context(|| "workproof_bad")?;
  *counter += 1;
  sequence_unknown_user::<M>()
      .await
      .with_context(|| "sequence_unknown_user")?;
  *counter += 1;
  sequence_multiple_problems::<M>()
      .await
      .with_context(|| "sequence_bad")?;
  *counter += 1;
  simple_client_test::<M>()
      .await
      .with_context(|| "simple_client_test")?;
  *counter += 1;
  list_users_test::<M>()
      .await
      .with_context(|| "list_users_test")?;
  *counter += 1;
  multiple_client_messages_test::<M>()
      .await
      .with_context(|| "multiple_client_message_test")?;
  *counter += 1;
  mixed_results_client_message::<M>()
      .await
      .with_context(|| "mixed_results_client_message")?;
  *counter += 1;
  mailbox_full::<M>().await.with_context(|| "mailbox_full")?;
  println!("All base tests passed");
  println!("Running federation tests...");
  *counter += 1;
  #[cfg(feature = "federation")]
  {
    message_to_outer_user::<M>()
        .await
        .with_context(|| "message_to_outer_user")?;
    println!("message_to_outer_user passed");
    *counter += 1;
    message_to_outer_user_delayed::<M>()
        .await
        .with_context(|| "message_to_outer_user_delayed")?;
    println!("message_to_outer_user_delayed passed");
    *counter += 1;
    routing_test::<M>().await.with_context(|| "routing")?;
    println!("routing passed");
    *counter += 1;
  }
  Ok(())
}

pub(crate) fn test_message_server<M: MessageServer>() {
  pretty_env_logger::init();
  async_std::task::block_on(async {
    let mut counter = 0;
    match all_tests::<M>(&mut counter).await {
      Ok(()) => (),
      Err(rr) => panic!("counter={}, error={:?}", counter, rr),
    }
  });
}