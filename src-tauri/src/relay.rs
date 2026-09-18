//! The signalling relay on this app's own origin: `/ws?room=…&role=…`.
//!
//! A port of the relay in Tawny Docker's server.js, and through it of
//! `rendezvous/protocol.js` — the same admission, close codes, caps and ticket
//! clock, so the page cannot tell which relay it is talking to. Keep the three
//! in step: RELAY, the CLOSE codes and the limits below are shared constants
//! there.
//!
//! The page reaches this as its relay of last resort (`sameOriginBase()` in
//! app.js). It is bound to loopback, so it only ever serves this machine.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::ws::{CloseFrame, Message, Utf8Bytes, WebSocket};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

// ---------------------------------------------------- rendezvous/protocol.js

/// Every relayed type is addressed; peer ids come from the relay.
const RELAY: &[&str] = &[
  "offer", "answer", "ice", "bye", "chime", "chime-ack", "talking",
  "cameras", "meta", "camera-control", "torch", "battery",
];
const MAX_PER_ROOM: usize = 4;
const MAX_STATIONS: usize = 1;
pub const MAX_MSG: usize = 64 * 1024;
const ADMIT_TIMEOUT: Duration = Duration::from_secs(10);
const TICKET_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const TICKET_MAX_LIFETIME: Duration = Duration::from_secs(30 * 24 * 60 * 60);

type Close = (u16, &'static str);
const EXPECTED_HELLO: Close = (4000, "expected hello");
const FULL: Close = (4003, "channel full");
const MONITOR_RUNNING: Close = (4004, "monitor already running");
const BUSY: Close = (4005, "busy");
const REPLACED: Close = (4005, "replaced by owner");
const NO_HELLO: Close = (4008, "no hello");
const PAIRING_EXPIRED: Close = (4008, "pairing expired");
const WRONG_KEY: Close = (4008, "wrong channel key");
const NO_TICKET: Close = (4008, "no pairing ticket");
const MONITOR_OFFLINE: Close = (4010, "monitor offline");

// ----------------------------------------------------------- server.js caps

pub const MAX_TOTAL: usize = 64;
const MAX_ROOMS: usize = 256;
const HEARTBEAT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, PartialEq)]
pub enum Role {
  Station,
  Viewer,
}

impl Role {
  pub fn parse(s: &str) -> Self {
    if s == "station" { Role::Station } else { Role::Viewer }
  }
  fn as_str(self) -> &'static str {
    match self {
      Role::Station => "station",
      Role::Viewer => "viewer",
    }
  }
}

enum Out {
  Text(String),
  Ping,
  Close(Close),
}

struct Peer {
  role: Role,
  tx: mpsc::UnboundedSender<Out>,
}

impl Peer {
  fn send(&self, v: &Value) {
    let _ = self.tx.send(Out::Text(v.to_string()));
  }
  fn close(&self, c: Close) {
    let _ = self.tx.send(Out::Close(c));
  }
}

/// room -> sha256(ticket) the Monitor registered, and optionally the proof of
/// the channel key it offered: the only thing that lets it reclaim its room.
struct Ticket {
  hash_t: String,
  auth: Option<String>,
  iss: Instant,
  exp: Instant,
}

impl Ticket {
  fn beyond_lifetime(&self, now: Instant) -> bool {
    now.duration_since(self.iss) >= TICKET_MAX_LIFETIME
  }
  fn live(&self, now: Instant) -> bool {
    now <= self.exp && !self.beyond_lifetime(now)
  }
}

#[derive(Default)]
struct Inner {
  rooms: HashMap<String, HashMap<String, Peer>>,
  tickets: HashMap<String, Ticket>,
  clients: usize,
}

#[derive(Clone, Default)]
pub struct Relay(Arc<Mutex<Inner>>);

fn sha256hex(s: &str) -> String {
  hex::encode(Sha256::digest(s.as_bytes()))
}

fn hex64(v: Option<&Value>) -> Option<String> {
  let s = v?.as_str()?;
  (s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))).then(|| s.to_string())
}

pub fn room_ok(room: &str) -> bool {
  room.len() == 32 && room.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn new_id() -> String {
  format!("{:08x}", rand::random::<u32>())
}

impl Relay {
  pub fn full(&self) -> bool {
    self.0.lock().unwrap().clients >= MAX_TOTAL
  }

  /// Roll tickets forward under a connected Monitor and drop dead ones, as
  /// server.js's heartbeat does. Runs for the life of the process.
  pub async fn upkeep(self) {
    let mut tick = tokio::time::interval(HEARTBEAT);
    loop {
      tick.tick().await;
      let now = Instant::now();
      let mut g = self.0.lock().unwrap();
      let Inner { rooms, tickets, .. } = &mut *g;
      tickets.retain(|room, rec| {
        let station_here = rooms
          .get(room)
          .is_some_and(|peers| peers.values().any(|p| p.role == Role::Station));
        if station_here && !rec.beyond_lifetime(now) {
          rec.exp = now + TICKET_TTL;
          true
        } else {
          rec.live(now)
        }
      });
    }
  }

  pub async fn serve(self, socket: WebSocket, room: String, role: Role) {
    let id = new_id();
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Out>();

    self.0.lock().unwrap().clients += 1;

    // Writer: everything addressed to this socket, from any task.
    let writer = tokio::spawn(async move {
      while let Some(out) = rx.recv().await {
        let msg = match out {
          Out::Text(s) => Message::Text(s.into()),
          Out::Ping => Message::Ping(Default::default()),
          Out::Close((code, reason)) => {
            let _ = sink
              .send(Message::Close(Some(CloseFrame { code, reason: Utf8Bytes::from_static(reason) })))
              .await;
            break;
          }
        };
        if sink.send(msg).await.is_err() {
          break;
        }
      }
    });

    // Say hello or go away.
    let admitted = match tokio::time::timeout(ADMIT_TIMEOUT, next_json(&mut stream)).await {
      Ok(Some(msg)) if msg.get("type").and_then(Value::as_str) == Some("hello") => {
        self.admit(&room, &id, role, &msg, &tx)
      }
      Ok(Some(_)) => {
        let _ = tx.send(Out::Close(EXPECTED_HELLO));
        false
      }
      Ok(None) => false,
      Err(_) => {
        let _ = tx.send(Out::Close(NO_HELLO));
        false
      }
    };

    if admitted {
      log::info!("+ {} {id} -> {}", role.as_str(), &room[..8]);
      self.pump(&mut stream, &room, &id, &tx).await;
    }

    self.leave(&room, &id);
    drop(tx);
    let _ = writer.await;
  }

  /// Relay addressed messages until the socket goes, answering the heartbeat.
  async fn pump(
    &self,
    stream: &mut futures_util::stream::SplitStream<WebSocket>,
    room: &str,
    id: &str,
    tx: &mpsc::UnboundedSender<Out>,
  ) {
    // server.js's heartbeat: ping every 30 s, and a socket that has not been
    // heard from since the previous ping — not even a pong — is gone.
    let mut last_seen = Instant::now();
    let mut tick = tokio::time::interval(HEARTBEAT);
    loop {
      tokio::select! {
        m = stream.next() => {
          let Some(Ok(m)) = m else { return };
          last_seen = Instant::now();
          let Message::Text(text) = m else {
            if matches!(m, Message::Close(_)) { return }
            continue;
          };
          let Ok(mut msg) = serde_json::from_str::<Value>(&text) else { continue };
          let Some(obj) = msg.as_object_mut() else { continue };
          let ty = obj.get("type").and_then(Value::as_str).unwrap_or("");
          if !RELAY.contains(&ty) { continue }
          let Some(to) = obj.get("to").and_then(Value::as_str).map(String::from) else { continue };
          if to == id { continue }
          obj.insert("from".into(), Value::String(id.to_string()));
          let g = self.0.lock().unwrap();
          if let Some(target) = g.rooms.get(room).and_then(|peers| peers.get(&to)) {
            target.send(&msg);
          }
        }
        _ = tick.tick() => {
          if last_seen.elapsed() > HEARTBEAT + Duration::from_secs(5) { return }
          let _ = tx.send(Out::Ping);
        }
      }
    }
  }

  /// server.js's admit(), step for step. Nothing that is rejected may change
  /// stored state, and the room entry is created only on success.
  fn admit(&self, room: &str, id: &str, role: Role, msg: &Value, tx: &mpsc::UnboundedSender<Out>) -> bool {
    let now = Instant::now();
    let mut g = self.0.lock().unwrap();
    let refuse = |c: Close| {
      let _ = tx.send(Out::Close(c));
      false
    };

    if g.tickets.get(room).is_some_and(|rec| !rec.live(now)) {
      g.tickets.remove(room);
    }
    let rec = g.tickets.get(room);
    let peers_len = g.rooms.get(room).map_or(0, HashMap::len);
    let proof = hex64(msg.get("a"));
    let ticket_hash = msg.get("t").and_then(Value::as_str).map(sha256hex);

    if peers_len >= MAX_PER_ROOM {
      return refuse(FULL);
    }
    if !g.rooms.contains_key(room) && g.rooms.len() >= MAX_ROOMS {
      return refuse(BUSY);
    }

    let mut evict: Vec<String> = Vec::new();
    if role == Role::Station {
      let stations: Vec<String> = g
        .rooms
        .get(room)
        .map(|peers| peers.iter().filter(|(_, p)| p.role == Role::Station).map(|(k, _)| k.clone()).collect())
        .unwrap_or_default();
      if stations.len() >= MAX_STATIONS {
        let owner = matches!((rec.and_then(|r| r.auth.as_ref()), &proof), (Some(a), Some(p)) if a == p);
        if !owner {
          return refuse(MONITOR_RUNNING);
        }
        evict = stations;
      }
    }

    let mut register: Option<Ticket> = None;
    match role {
      Role::Viewer => {
        let Some(rec) = rec else { return refuse(MONITOR_OFFLINE) };
        if ticket_hash.as_deref() != Some(rec.hash_t.as_str()) {
          return refuse(PAIRING_EXPIRED);
        }
      }
      Role::Station => {
        let rec_auth = rec.and_then(|r| r.auth.clone());
        if let (Some(a), Some(p)) = (&rec_auth, &proof)
          && a != p
        {
          return refuse(WRONG_KEY);
        }
        let may_rekey = proof.is_some() || rec_auth.is_none();
        let hash_t = hex64(msg.get("hashT"));
        if let (Some(hash_t), true) = (hash_t, may_rekey) {
          let iss = match rec {
            Some(r) if r.hash_t == hash_t => r.iss,
            _ => now,
          };
          register = Some(Ticket { hash_t, auth: proof.clone().or(rec_auth), iss, exp: now + TICKET_TTL });
        } else if let Some(rec) = rec {
          if ticket_hash.as_deref() != Some(rec.hash_t.as_str()) {
            return refuse(PAIRING_EXPIRED);
          }
        } else {
          return refuse(NO_TICKET);
        }
      }
    }

    // Admitted. Only now may the ticket move, or a sitting Monitor be hung up on.
    if let Some(t) = register {
      g.tickets.insert(room.to_string(), t);
    }
    let peers = g.rooms.entry(room.to_string()).or_default();
    for old in evict {
      if let Some(p) = peers.remove(&old) {
        p.close(REPLACED);
      }
      for peer in peers.values() {
        peer.send(&json!({ "type": "peer-left", "id": old }));
      }
    }
    let others: Vec<Value> = peers
      .iter()
      .map(|(pid, p)| json!({ "id": pid, "role": p.role.as_str() }))
      .collect();
    for peer in peers.values() {
      peer.send(&json!({ "type": "peer-joined", "id": id, "role": role.as_str() }));
    }
    let me = Peer { role, tx: tx.clone() };
    me.send(&json!({ "type": "welcome", "id": id, "role": role.as_str(), "peers": others }));
    peers.insert(id.to_string(), me);
    true
  }

  fn leave(&self, room: &str, id: &str) {
    let mut g = self.0.lock().unwrap();
    g.clients -= 1;
    let Some(peers) = g.rooms.get_mut(room) else { return };
    if peers.remove(id).is_some() {
      log::info!("- {id} <- {} ({})", &room[..8], peers.len());
      for peer in peers.values() {
        peer.send(&json!({ "type": "peer-left", "id": id }));
      }
    }
    if peers.is_empty() {
      g.rooms.remove(room);
    }
  }
}

async fn next_json(stream: &mut futures_util::stream::SplitStream<WebSocket>) -> Option<Value> {
  loop {
    match stream.next().await? {
      // Like server.js: a frame that is not a JSON object is ignored, not refused.
      Ok(Message::Text(t)) => match serde_json::from_str::<Value>(&t) {
        Ok(v) if v.is_object() => return Some(v),
        _ => continue,
      },
      Ok(Message::Close(_)) | Err(_) => return None,
      Ok(_) => continue,
    }
  }
}
