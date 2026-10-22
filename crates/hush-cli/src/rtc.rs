//! One WebRTC link, as in `web/src/net/peer.ts`: three negotiated DataChannels (ctrl 0, chat 1,
//! file 2), non-trickle ICE (the description is complete once gathering ends). The SDP only ever
//! travels inside the post-quantum handshake, and every message on it is already a sealed
//! session frame.
//!
//! Each link runs as an actor task, driven by `RtcCmd` and reporting `RtcEvent`.

use anyhow::{Result, anyhow};
use bytes::BytesMut;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use webrtc::data_channel::{DataChannel, DataChannelEvent, RTCDataChannelInit};
use webrtc::media_stream::track_remote::TrackRemote;
use webrtc::peer_connection::{
    MediaEngine, PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler, RTCConfigurationBuilder,
    RTCIceGatheringState, RTCIceServer, RTCIceTransportPolicy, RTCPeerConnectionState, RTCSessionDescription,
    SettingEngine,
};

pub const LABELS: [&str; 3] = ["ctrl", "chat", "file"];
const GATHER_TIMEOUT: Duration = Duration::from_secs(4);

pub enum RtcCmd {
    CreateOffer(oneshot::Sender<Result<String>>),
    AcceptOffer(String, oneshot::Sender<Result<String>>),
    AcceptAnswer(String),
    Send(usize, Vec<u8>),
    /// Like `Send`, then reports `Sent` once the frame is queued (file transfers pace on it).
    SendTracked(usize, Vec<u8>),
    Close,
}

#[derive(Debug)]
pub enum RtcEvent {
    Open,
    Frame(Vec<u8>),
    Sent,
    Closed,
}

enum Internal {
    Gathering(RTCIceGatheringState),
    Conn(RTCPeerConnectionState),
    Dc(usize, DataChannelEvent),
}

struct Handler {
    tx: mpsc::UnboundedSender<Internal>,
}

#[async_trait::async_trait]
impl PeerConnectionEventHandler for Handler {
    async fn on_ice_gathering_state_change(&self, state: RTCIceGatheringState) {
        let _ = self.tx.send(Internal::Gathering(state));
    }
    async fn on_connection_state_change(&self, state: RTCPeerConnectionState) {
        let _ = self.tx.send(Internal::Conn(state));
    }
    async fn on_track(&self, track: Arc<dyn TrackRemote>) {
        // hush never carries media: drain it so the driver is never blocked on us.
        tokio::spawn(async move { while track.poll().await.is_some() {} });
    }
}

pub struct RtcOptions {
    pub ice_servers: Vec<RTCIceServer>,
    pub relay_only: bool,
}

/// Spawns a link; events are tagged with `key`.
pub fn spawn<K: Copy + Send + 'static>(
    key: K,
    opts: RtcOptions,
    events: mpsc::UnboundedSender<(K, RtcEvent)>,
) -> mpsc::UnboundedSender<RtcCmd> {
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let ev = events.clone();
        if run(key, opts, cmd_rx, events).await.is_err() {
            let _ = ev.send((key, RtcEvent::Closed));
        }
    });
    cmd_tx
}

struct Actor {
    pc: Arc<dyn PeerConnection>,
    dcs: Vec<Arc<dyn DataChannel>>,
    open: [bool; 3],
    reported: bool,
    gathered: bool,
    internal_tx: mpsc::UnboundedSender<Internal>,
}

async fn run<K: Copy + Send + 'static>(
    key: K,
    opts: RtcOptions,
    mut cmds: mpsc::UnboundedReceiver<RtcCmd>,
    events: mpsc::UnboundedSender<(K, RtcEvent)>,
) -> Result<()> {
    let (internal_tx, mut internal) = mpsc::unbounded_channel();
    let mut media = MediaEngine::default();
    media.register_default_codecs()?;
    let mut setting = SettingEngine::default();
    // Browsers send messages up to 256 KiB.
    setting.set_sctp_max_message_size(
        rtc::peer_connection::configuration::setting_engine::SctpMaxMessageSize::Bounded(262_144),
    );
    let mut config = RTCConfigurationBuilder::new().with_ice_servers(opts.ice_servers);
    if opts.relay_only {
        config = config.with_ice_transport_policy(RTCIceTransportPolicy::Relay);
    }
    let pc = PeerConnectionBuilder::new()
        .with_configuration(config.build())
        .with_media_engine(media)
        .with_setting_engine(setting)
        .with_handler(Arc::new(Handler {
            tx: internal_tx.clone(),
        }))
        .with_udp_addrs(vec!["0.0.0.0:0".to_string()])
        .with_data_channel_send_buffer_limit(16 * 1024 * 1024)
        .build()
        .await?;
    let mut a = Actor {
        pc: Arc::new(pc),
        dcs: Vec::new(),
        open: [false; 3],
        reported: false,
        gathered: false,
        internal_tx,
    };
    a.create_channels().await?;
    loop {
        tokio::select! {
            cmd = cmds.recv() => match cmd {
                None | Some(RtcCmd::Close) => break,
                Some(RtcCmd::CreateOffer(reply)) => {
                    let r = a.offer(&mut internal).await;
                    let _ = reply.send(r);
                }
                Some(RtcCmd::AcceptOffer(sdp, reply)) => {
                    let r = a.answer(sdp, &mut internal).await;
                    let _ = reply.send(r);
                }
                Some(RtcCmd::AcceptAnswer(sdp)) => {
                    if let Ok(desc) = RTCSessionDescription::answer(sdp) {
                        let _ = a.pc.set_remote_description(desc).await;
                    }
                }
                Some(RtcCmd::Send(lane, frame)) => {
                    if let Some(dc) = a.dcs.get(lane)
                        && a.open[lane] { let _ = dc.send(BytesMut::from(&frame[..])).await; }
                }
                Some(RtcCmd::SendTracked(lane, frame)) => {
                    if let Some(dc) = a.dcs.get(lane)
                        && a.open[lane] { let _ = dc.send(BytesMut::from(&frame[..])).await; }
                    let _ = events.send((key, RtcEvent::Sent));
                }
            },
            ev = internal.recv() => match ev {
                None => break,
                Some(Internal::Gathering(s)) => a.gathered = s == RTCIceGatheringState::Complete,
                Some(Internal::Conn(RTCPeerConnectionState::Failed | RTCPeerConnectionState::Closed)) => break,
                Some(Internal::Conn(_)) => {}
                Some(Internal::Dc(i, DataChannelEvent::OnOpen)) => {
                    a.open[i] = true;
                    if !a.reported && a.open.iter().all(|o| *o) {
                        a.reported = true;
                        let _ = events.send((key, RtcEvent::Open));
                    }
                }
                Some(Internal::Dc(_, DataChannelEvent::OnClose)) => break,
                Some(Internal::Dc(_, DataChannelEvent::OnMessage(msg))) => {
                    if !msg.is_string { let _ = events.send((key, RtcEvent::Frame(msg.data.to_vec()))); }
                }
                Some(Internal::Dc(..)) => {}
            },
        }
    }
    let _ = a.pc.close().await;
    let _ = events.send((key, RtcEvent::Closed));
    Ok(())
}

impl Actor {
    async fn create_channels(&mut self) -> Result<()> {
        for (i, label) in LABELS.iter().enumerate() {
            let init = RTCDataChannelInit {
                ordered: true,
                negotiated: Some(u16::try_from(i).expect("3 lanes")),
                ..Default::default()
            };
            let dc = self.pc.create_data_channel(label, Some(init)).await?;
            let tx = self.internal_tx.clone();
            let poll = dc.clone();
            tokio::spawn(async move {
                while let Some(ev) = poll.poll().await {
                    if tx.send(Internal::Dc(i, ev)).is_err() {
                        break;
                    }
                }
            });
            self.dcs.push(dc);
        }
        Ok(())
    }

    async fn wait_gathered(&mut self, internal: &mut mpsc::UnboundedReceiver<Internal>) {
        if self.gathered {
            return;
        }
        let deadline = tokio::time::sleep(GATHER_TIMEOUT);
        tokio::pin!(deadline);
        let mut deferred = Vec::new();
        loop {
            tokio::select! {
                _ = &mut deadline => break,
                ev = internal.recv() => match ev {
                    Some(Internal::Gathering(RTCIceGatheringState::Complete)) => { self.gathered = true; break; }
                    Some(other) => deferred.push(other),
                    None => break,
                },
            }
        }
        for ev in deferred {
            let _ = self.internal_tx.send(ev);
        }
    }

    async fn local_sdp(&self) -> Result<String> {
        self.pc
            .local_description()
            .await
            .map(|d| d.sdp)
            .ok_or_else(|| anyhow!("no local description"))
    }

    async fn offer(&mut self, internal: &mut mpsc::UnboundedReceiver<Internal>) -> Result<String> {
        let offer = self.pc.create_offer(None).await?;
        self.pc.set_local_description(offer).await?;
        self.wait_gathered(internal).await;
        self.local_sdp().await
    }

    async fn answer(&mut self, sdp: String, internal: &mut mpsc::UnboundedReceiver<Internal>) -> Result<String> {
        self.pc
            .set_remote_description(RTCSessionDescription::offer(sdp)?)
            .await?;
        let answer = self.pc.create_answer(None).await?;
        self.pc.set_local_description(answer).await?;
        self.wait_gathered(internal).await;
        self.local_sdp().await
    }
}
