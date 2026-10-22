//! Test harness: engines wired by simulated nearby links (what the phones' Bluetooth and Wi-Fi
//! provide), and a minimal Nostr relay on localhost.
#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use hush_app::signal::{LinkId, SignalConfig};
use hush_app::{App, Config, Event, LocalHost, LocalIn, LocalKind, LocalMedium};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

pub fn config(name: &str, out: PathBuf, data: Option<PathBuf>, relays: Vec<String>) -> Config {
    Config {
        name: name.into(),
        base: "https://maxgfr.github.io/hush/".into(),
        signal: SignalConfig {
            relays,
            trackers: vec![],
        },
        ice_servers: vec![],
        relay_only: false,
        out_dir: out,
        data_dir: data,
        local: true,
        ttl: 0,
    }
}

/// Every simulated device, so they can find each other.
#[derive(Default)]
pub struct Air {
    devices: Mutex<Vec<Arc<Device>>>,
    /// Pairs already linked for a token: (scanner, advertiser, token).
    linked: Mutex<HashSet<(usize, usize, Vec<u8>)>>,
    next: AtomicU64,
    pub kind: Mutex<Option<LocalKind>>,
}

pub struct Device {
    index: usize,
    air: Arc<Air>,
    app: OnceLock<App>,
    advertising: Mutex<Vec<Vec<u8>>>,
    scanning: Mutex<Vec<Vec<u8>>>,
    /// Our link id → (other device, its link id).
    ends: Mutex<HashMap<LinkId, (usize, LinkId)>>,
}

impl Air {
    pub fn new(kind: LocalKind) -> Arc<Self> {
        let air = Arc::new(Self::default());
        *air.kind.lock().unwrap() = Some(kind);
        air
    }

    pub fn device(self: &Arc<Self>) -> Arc<Device> {
        let mut devices = self.devices.lock().unwrap();
        let d = Arc::new(Device {
            index: devices.len(),
            air: self.clone(),
            app: OnceLock::new(),
            advertising: Mutex::new(Vec::new()),
            scanning: Mutex::new(Vec::new()),
            ends: Mutex::new(HashMap::new()),
        });
        devices.push(d.clone());
        d
    }

    fn get(&self, i: usize) -> Arc<Device> {
        self.devices.lock().unwrap()[i].clone()
    }

    /// Connects any scanner to any advertiser of a token it looks for, as a phone would.
    fn rescan(&self) {
        let devices = self.devices.lock().unwrap().clone();
        let kind = self.kind.lock().unwrap().expect("set");
        for s in &devices {
            let wanted = s.scanning.lock().unwrap().clone();
            for a in &devices {
                if a.index == s.index {
                    continue;
                }
                let offered = a.advertising.lock().unwrap().clone();
                for t in wanted.iter().filter(|t| offered.contains(t)) {
                    if !self.linked.lock().unwrap().insert((s.index, a.index, t.clone())) {
                        continue;
                    }
                    let (ls, la) = (
                        self.next.fetch_add(1, Ordering::SeqCst) + 1,
                        self.next.fetch_add(1, Ordering::SeqCst) + 1,
                    );
                    s.ends.lock().unwrap().insert(ls, (a.index, la));
                    a.ends.lock().unwrap().insert(la, (s.index, ls));
                    let medium = if matches!(kind, LocalKind::Stream) {
                        LocalMedium::Wifi
                    } else {
                        LocalMedium::Bluetooth
                    };
                    a.app().local(LocalIn::Opened {
                        link: la,
                        kind,
                        medium,
                        token: None,
                    });
                    s.app().local(LocalIn::Opened {
                        link: ls,
                        kind,
                        medium,
                        token: Some(t.clone()),
                    });
                }
            }
        }
    }
}

impl Device {
    pub fn attach(&self, app: App) {
        let _ = self.app.set(app);
    }

    fn app(&self) -> &App {
        self.app.get().expect("attached")
    }

    /// Simulates walking away: every link of this device drops.
    pub fn drop_links(&self) {
        let ends: Vec<(LinkId, (usize, LinkId))> = self.ends.lock().unwrap().drain().collect();
        for (mine, (other, theirs)) in ends {
            self.app().local(LocalIn::Closed { link: mine });
            let o = self.air.get(other);
            o.ends.lock().unwrap().remove(&theirs);
            o.app().local(LocalIn::Closed { link: theirs });
        }
        self.air
            .linked
            .lock()
            .unwrap()
            .retain(|(s, a, _)| *s != self.index && *a != self.index);
    }
}

impl LocalHost for Device {
    fn advertise(&self, tokens: Vec<Vec<u8>>) {
        *self.advertising.lock().unwrap() = tokens;
        self.air.rescan();
    }

    fn scan(&self, tokens: Vec<Vec<u8>>) {
        *self.scanning.lock().unwrap() = tokens;
        self.air.rescan();
    }

    fn send(&self, link: LinkId, data: Vec<u8>) {
        let Some((other, theirs)) = self.ends.lock().unwrap().get(&link).copied() else {
            return;
        };
        let n = data.len();
        self.air.get(other).app().local(LocalIn::Data { link: theirs, data });
        self.app().local(LocalIn::Sent { link, bytes: n });
    }

    fn close(&self, link: LinkId) {
        if let Some((other, theirs)) = self.ends.lock().unwrap().remove(&link) {
            let o = self.air.get(other);
            o.ends.lock().unwrap().remove(&theirs);
            o.app().local(LocalIn::Closed { link: theirs });
        }
    }
}

/// Collects one engine's events and waits for the ones a test expects.
pub struct Events {
    rx: mpsc::UnboundedReceiver<Event>,
    pub seen: Vec<Event>,
}

impl Events {
    pub fn new(rx: mpsc::UnboundedReceiver<Event>) -> Self {
        Self { rx, seen: Vec::new() }
    }

    /// The first event (already seen or coming) that matches, within 30 s.
    pub async fn wait<T>(&mut self, what: &str, pick: impl FnMut(&Event) -> Option<T>) -> T {
        self.wait_for(what, Duration::from_secs(30), pick).await
    }

    /// `wait` with its own patience (a proof of work takes a while in a debug build on CI).
    pub async fn wait_for<T>(
        &mut self,
        what: &str,
        patience: Duration,
        mut pick: impl FnMut(&Event) -> Option<T>,
    ) -> T {
        if let Some(t) = self.seen.iter().find_map(&mut pick) {
            return t;
        }
        let deadline = tokio::time::Instant::now() + patience;
        loop {
            let ev = tokio::time::timeout_at(deadline, self.rx.recv())
                .await
                .unwrap_or_else(|_| panic!("timed out waiting for {what}; saw {:#?}", self.seen))
                .expect("engine alive");
            let hit = pick(&ev);
            self.seen.push(ev);
            if let Some(t) = hit {
                return t;
            }
        }
    }

    /// Forgets what was seen (to wait for a second occurrence).
    pub fn clear(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            self.seen.push(ev);
        }
        self.seen.clear();
    }
}

/// A Nostr relay that does just what hush needs: ephemeral events of kind 21918, `#t` filters.
pub async fn relay() -> String {
    relay_with_tap().await.0
}

/// The relay, and every event published on it as `(topic hex, content)`: what anyone subscribed
/// to the kind sees.
pub async fn relay_with_tap() -> (String, mpsc::UnboundedReceiver<(String, String)>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (bus, _) = tokio::sync::broadcast::channel::<Value>(4096);
    let (tap, tapped) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let bus = bus.clone();
            let tap = tap.clone();
            tokio::spawn(async move {
                let Ok(ws) = tokio_tungstenite::accept_async(stream).await else {
                    return;
                };
                let (mut tx, mut rx) = ws.split();
                let mut events = bus.subscribe();
                let mut subs: HashMap<String, Vec<String>> = HashMap::new();
                loop {
                    tokio::select! {
                        msg = rx.next() => {
                            let Some(Ok(Message::Text(t))) = msg else { break };
                            let Ok(v) = serde_json::from_str::<Value>(t.as_str()) else { continue };
                            match v[0].as_str() {
                                Some("REQ") => {
                                    let topics = v[2]["#t"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
                                    subs.insert(v[1].as_str().unwrap_or("").to_string(), topics);
                                }
                                Some("CLOSE") => { subs.remove(v[1].as_str().unwrap_or("")); }
                                Some("EVENT") => {
                                    let ev = &v[1];
                                    let tagged = (ev["tags"][0][1].as_str(), ev["content"].as_str());
                                    if let (Some(topic), Some(content)) = tagged {
                                        let _ = tap.send((topic.to_string(), content.to_string()));
                                    }
                                    let _ = bus.send(ev.clone());
                                }
                                _ => {}
                            }
                        }
                        ev = events.recv() => {
                            let Ok(ev) = ev else { continue };
                            let topic = ev["tags"][0][1].as_str().unwrap_or("").to_string();
                            for (id, topics) in &subs {
                                if topics.contains(&topic) {
                                    let _ = tx.send(Message::text(json!(["EVENT", id, ev]).to_string())).await;
                                }
                            }
                        }
                    }
                }
            });
        }
    });
    (format!("ws://{addr}"), tapped)
}
