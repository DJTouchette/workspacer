//! Engine-owned conversation fragments and status lines on the shared bus.
//! Ready is a broker-committed generation barrier, not a subscribe acknowledgement.
use crate::{Handle, Options, protocol::Event};
use anyhow::{Result, bail};
use claudemon::{
    daemon::embedded::EmbeddedClient,
    session::{conversation::ConversationDelta, store::StatusLineUpdate},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, RwLock},
    time::Duration,
};
use tokio::sync::{Notify, broadcast, watch};
const PREFIX: &str = "agent.conversation.";
const MAX_WANTED: usize = 4096;
#[derive(Clone, Debug)]
enum Kind {
    Ready,
    Delta,
    StatusLine,
}
/// Private host producer envelope. Wire callers cannot construct a broker command.
#[derive(Clone, Debug)]
pub struct Delivery {
    session: String,
    generation: u64,
    kind: Kind,
    data: Value,
}
trait Source: Send + Sync + 'static {
    fn conversations(&self) -> Result<broadcast::Receiver<ConversationDelta>>;
    fn status_lines(&self) -> Result<broadcast::Receiver<StatusLineUpdate>>;
    fn available(&self) -> bool;
}
struct EngineSource(EmbeddedClient);
impl Source for EngineSource {
    fn conversations(&self) -> Result<broadcast::Receiver<ConversationDelta>> {
        self.0.subscribe_conversations()
    }
    fn status_lines(&self) -> Result<broadcast::Receiver<StatusLineUpdate>> {
        self.0.subscribe_status_lines()
    }
    fn available(&self) -> bool {
        matches!(
            *self.0.status().borrow(),
            claudemon::daemon::embedded::Status::Ready(_)
        )
    }
}
struct Demand {
    generation: u64,
    ready_queued: bool,
    active: bool,
}
struct State {
    wanted: BTreeMap<String, Demand>,
    next: u64,
    stream_epoch: u64,
    subscribed: bool,
    closed: bool,
}
pub struct LiveStreams {
    source: Arc<dyn Source>,
    rows: Arc<RwLock<BTreeMap<String, Value>>>,
    layout: Option<Arc<super::layout::Layout>>,
    upstream_layout: Option<Arc<RwLock<Value>>>,
    state: Mutex<State>,
    changed: Notify,
    closed: watch::Sender<bool>,
}
impl LiveStreams {
    pub fn new(
        engine: EmbeddedClient,
        rows: Arc<RwLock<BTreeMap<String, Value>>>,
        layout: Option<Arc<super::layout::Layout>>,
        upstream_layout: Option<Arc<RwLock<Value>>>,
    ) -> Arc<Self> {
        Self::from_source(
            Arc::new(EngineSource(engine)),
            rows,
            layout,
            upstream_layout,
        )
    }
    fn from_source(
        source: Arc<dyn Source>,
        rows: Arc<RwLock<BTreeMap<String, Value>>>,
        layout: Option<Arc<super::layout::Layout>>,
        upstream_layout: Option<Arc<RwLock<Value>>>,
    ) -> Arc<Self> {
        let (closed, _) = watch::channel(false);
        Arc::new(Self {
            source,
            rows,
            layout,
            upstream_layout,
            state: Mutex::new(State {
                wanted: BTreeMap::new(),
                next: 0,
                stream_epoch: 0,
                subscribed: false,
                closed: false,
            }),
            changed: Notify::new(),
            closed,
        })
    }
    /// Called synchronously by the broker for exact-topic0-to1/1-to0 transitions.
    pub fn set_demand(&self, topic: &str, on: bool) {
        let Some(id) = topic
            .strip_prefix(PREFIX)
            .filter(|id| !id.is_empty() && !id.contains('*') && id.len() <= 256)
        else {
            return;
        };
        let mut state = self.state.lock().unwrap();
        if state.closed {
            return;
        }
        if on {
            if state.wanted.contains_key(id) || state.wanted.len() >= MAX_WANTED {
                return;
            }
            if state.wanted.is_empty() {
                state.stream_epoch = state.stream_epoch.wrapping_add(1);
                state.subscribed = false;
            }
            state.next = state
                .next
                .checked_add(1)
                .expect("stream demand generation exhausted");
            let generation = state.next;
            state.wanted.insert(
                id.into(),
                Demand {
                    generation,
                    ready_queued: false,
                    active: false,
                },
            );
        } else {
            state.wanted.remove(id);
            if state.wanted.is_empty() {
                state.stream_epoch = state.stream_epoch.wrapping_add(1);
                state.subscribed = false;
            }
        }
        drop(state);
        self.changed.notify_one();
    }
    pub fn close(&self) {
        let mut state = self.state.lock().unwrap();
        state.closed = true;
        state.wanted.clear();
        state.subscribed = false;
        drop(state);
        self.closed.send_replace(true);
        self.changed.notify_one();
    }
    fn visible(&self, id: &str) -> bool {
        let Some(row) = self.rows.read().unwrap().get(id).cloned() else {
            return false;
        };
        if row["hub"].as_str().is_some_and(|s| !s.is_empty()) {
            return false;
        }
        let layout = self
            .upstream_layout
            .as_ref()
            .map(|l| l.read().unwrap().clone())
            .or_else(|| self.layout.as_ref().map(|l| l.get()))
            .unwrap_or(Value::Null);
        super::snapshots::visible(&row, &layout, time::OffsetDateTime::now_utc())
    }
    /// Must execute in Core immediately before Core.publish. This final check
    /// fences already-queued work after off/on, dismissal, hide, lag, or shutdown.
    pub fn accept_delivery(&self, delivery: Delivery) -> Option<Event> {
        if !self.source.available() {
            return None;
        }
        let mut state = self.state.lock().unwrap();
        if state.closed {
            return None;
        }
        if matches!(delivery.kind, Kind::StatusLine) {
            // A status update may refresh an admitted row; it must never invent one.
            {
                let mut rows = self.rows.write().unwrap();
                let row = rows.get_mut(&delivery.session)?;
                if !row.is_object() || row["hub"].as_str().is_some_and(|s| !s.is_empty()) {
                    return None;
                }
                row["status_line"] = delivery.data.clone();
                let mapped = super::snapshots::compat(
                    json!({"session_id":delivery.session,"status_line":delivery.data}),
                );
                row["statusLine"] = mapped["statusLine"].clone();
            }
            if !self.visible(&delivery.session) {
                return None;
            }
            return Some(Event::new(
                "agent.statusline",
                "brain",
                json!({"sessionId":delivery.session,"statusLine":delivery.data}),
            ));
        }
        if !state.subscribed {
            return None;
        }
        let demand = state.wanted.get_mut(&delivery.session)?;
        if demand.generation != delivery.generation {
            return None;
        }
        if !self.visible(&delivery.session) {
            demand.active = false;
            demand.ready_queued = false;
            return None;
        }
        match delivery.kind {
            Kind::Ready => {
                if demand.active {
                    return None;
                }
                demand.active = true;
                Some(Event::new(
                    format!("{PREFIX}{}", delivery.session),
                    "brain",
                    json!({"session_id":delivery.session,"ready":true}),
                ))
            }
            Kind::Delta if demand.active => Some(Event::new(
                format!("{PREFIX}{}", delivery.session),
                "brain",
                delivery.data,
            )),
            _ => None,
        }
    }
    fn pending_ready(&self, id: &str, generation: u64) -> Option<Delivery> {
        let mut state = self.state.lock().unwrap();
        if state.closed || !state.subscribed {
            return None;
        }
        let demand = state.wanted.get_mut(id)?;
        if demand.generation != generation
            || demand.active
            || demand.ready_queued
            || !self.visible(id)
        {
            return None;
        }
        demand.ready_queued = true;
        Some(Delivery {
            session: id.into(),
            generation,
            kind: Kind::Ready,
            data: Value::Null,
        })
    }
    fn invalidate_stream(&self) {
        let mut state = self.state.lock().unwrap();
        state.stream_epoch = state.stream_epoch.wrapping_add(1);
        state.subscribed = false;
        let mut next = state.next;
        for demand in state.wanted.values_mut() {
            next = next
                .checked_add(1)
                .expect("stream demand generation exhausted");
            demand.generation = next;
            demand.ready_queued = false;
            demand.active = false;
        }
        state.next = next;
        drop(state);
        self.changed.notify_one();
    }
    async fn send(&self, hub: &Handle, delivery: Delivery) -> Result<()> {
        let mut closed = self.closed.subscribe();
        if *closed.borrow() {
            return Ok(());
        }
        tokio::select! {biased;_=closed.changed()=>Ok(()),result=hub.publish_live_stream(delivery)=>result}
    }
    async fn announce(&self, hub: &Handle) -> Result<()> {
        let wanted: Vec<_> = self
            .state
            .lock()
            .unwrap()
            .wanted
            .iter()
            .map(|(id, d)| (id.clone(), d.generation))
            .collect();
        for (id, generation) in wanted {
            if let Some(delivery) = self.pending_ready(&id, generation) {
                self.send(hub, delivery).await?;
            }
        }
        Ok(())
    }
    pub async fn run(self: Arc<Self>, hub: Handle) -> Result<()> {
        let mut closed = self.closed.subscribe();
        if *closed.borrow() {
            return Ok(());
        }
        tokio::select! {biased;_=closed.changed()=>return Ok(()),ready=hub.ready()=>{ready?;}}
        let mut status = self.source.status_lines()?;
        let mut conversation: Option<(u64, broadcast::Receiver<ConversationDelta>)> = None;
        let mut visibility = tokio::time::interval(Duration::from_millis(100));
        visibility.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            if *closed.borrow() {
                return Ok(());
            }
            let (wanted, epoch) = {
                let state = self.state.lock().unwrap();
                (!state.wanted.is_empty(), state.stream_epoch)
            };
            if conversation
                .as_ref()
                .is_some_and(|(old, _)| !wanted || *old != epoch)
            {
                conversation = None;
            }
            if wanted && conversation.is_none() {
                let receiver = self.source.conversations()?;
                let mut state = self.state.lock().unwrap();
                if !state.closed && !state.wanted.is_empty() && state.stream_epoch == epoch {
                    state.subscribed = true;
                    conversation = Some((epoch, receiver));
                }
            }
            self.announce(&hub).await?;
            // Fair selection keeps status telemetry progressing during sustained
            // conversation traffic; the loop's stop check remains unconditional.
            tokio::select! {
             _=closed.changed()=>return Ok(()),
             _=self.changed.notified()=>(),
             _=visibility.tick()=>{if !self.source.available(){bail!("embedded live stream producer stopped");}
              // A hidden interval ends the ready contract. Reappearance needs a new
              // handshake even without an unsubscribe from the consumer.
              let mut state=self.state.lock().unwrap();for(id,demand)in &mut state.wanted{if !self.visible(id){demand.active=false;demand.ready_queued=false;}}
             },
             delta=async{match &mut conversation{Some((_,rx))=>rx.recv().await,None=>std::future::pending().await}},if conversation.is_some()=>match delta {
              Ok(delta)=>{
               let generation=self.state.lock().unwrap().wanted.get(&delta.session_id).map(|d|d.generation);
               if let Some(generation)=generation {
                if let Some(ready)=self.pending_ready(&delta.session_id,generation){self.send(&hub,ready).await?;}
                let delivery=Delivery{session:delta.session_id.clone(),generation,kind:Kind::Delta,data:serde_json::to_value(delta)?};self.send(&hub,delivery).await?;
               }
              },
              Err(broadcast::error::RecvError::Lagged(_))=>self.invalidate_stream(),
              Err(broadcast::error::RecvError::Closed)=>bail!("embedded conversation producer closed"),
             },
             line=status.recv()=>match line {
              Ok(line)=>self.send(&hub,Delivery{session:line.session_id,generation:0,kind:Kind::StatusLine,data:serde_json::to_value(line.status_line)?}).await?,
              // This channel has no invented reset schema. Drop stale buffered telemetry;
              // the next real sample repairs the last-observed value, with its own clock.
              Err(broadcast::error::RecvError::Lagged(_))=>{status=self.source.status_lines()?;},
              Err(broadcast::error::RecvError::Closed)=>bail!("embedded status-line producer closed"),
             }
            }
        }
    }
}
pub fn install(mut options: Options) -> Options {
    if let Some(engine) = options.engine.clone() {
        options.live_streams = Some(LiveStreams::new(
            engine,
            options.session_snapshots.clone(),
            options.layout.clone(),
            options.upstream_layout.clone(),
        ));
    }
    options
}

#[cfg(test)]
mod tests {
    use super::*;
    use claudemon::session::{conversation::ConversationItem, state::StatusLine};
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct FixtureSource {
        delta: broadcast::Sender<ConversationDelta>,
        status: broadcast::Sender<StatusLineUpdate>,
        initial: Mutex<Vec<ConversationDelta>>,
        starts: AtomicUsize,
    }
    impl FixtureSource {
        fn new(capacity: usize, initial: Vec<ConversationDelta>) -> Arc<Self> {
            let (delta, _) = broadcast::channel(capacity);
            let (status, _) = broadcast::channel(32);
            Arc::new(Self {
                delta,
                status,
                initial: Mutex::new(initial),
                starts: AtomicUsize::new(0),
            })
        }
    }
    impl Source for FixtureSource {
        fn conversations(&self) -> Result<broadcast::Receiver<ConversationDelta>> {
            self.starts.fetch_add(1, Ordering::SeqCst);
            let rx = self.delta.subscribe();
            for delta in std::mem::take(&mut *self.initial.lock().unwrap()) {
                let _ = self.delta.send(delta);
            }
            Ok(rx)
        }
        fn status_lines(&self) -> Result<broadcast::Receiver<StatusLineUpdate>> {
            Ok(self.status.subscribe())
        }
        fn available(&self) -> bool {
            true
        }
    }
    fn delta(id: &str, seq: u64, text: &str) -> ConversationDelta {
        ConversationDelta {
            session_id: id.into(),
            seq,
            reset: seq == 1,
            items: vec![ConversationItem::AssistantText {
                text: text.into(),
                timestamp: None,
            }],
        }
    }
    async fn until(mut condition: impl FnMut() -> bool) {
        tokio::time::timeout(Duration::from_secs(3), async {
            while !condition() {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .unwrap();
    }
    async fn next(events: &mut broadcast::Receiver<Event>) -> Event {
        tokio::time::timeout(Duration::from_secs(3), events.recv())
            .await
            .unwrap()
            .unwrap()
    }
    async fn rig(
        source: Arc<FixtureSource>,
        rows: BTreeMap<String, Value>,
        upstream: Option<Arc<RwLock<Value>>>,
    ) -> (
        crate::Hub,
        Arc<LiveStreams>,
        crate::client::Client,
        broadcast::Receiver<Event>,
    ) {
        let rows = Arc::new(RwLock::new(rows));
        let service = LiveStreams::from_source(source.clone(), rows.clone(), None, upstream);
        let mut options = Options::default();
        options.session_snapshots = rows;
        options.live_streams = Some(service.clone());
        let hub = crate::Hub::start(options).unwrap();
        hub.ready().await.unwrap();
        let client = crate::client::Client::connect(&hub.handle()).await.unwrap();
        let events = client.events();
        until(|| source.status.receiver_count() == 1).await;
        (hub, service, client, events)
    }
    fn shown() -> BTreeMap<String, Value> {
        BTreeMap::from([
            ("s1".into(), json!({"session_id":"s1","mode":"input"})),
            ("s2".into(), json!({"session_id":"s2","mode":"input"})),
        ])
    }
    #[tokio::test]
    async fn demand_owns_one_receiver_and_ready_precedes_already_buffered_delta() {
        let initial = delta("s1", 1, "first");
        let source = FixtureSource::new(8, vec![initial.clone()]);
        let (hub, _, client, mut events) = rig(source.clone(), shown(), None).await;
        assert_eq!(source.delta.receiver_count(), 0);
        client
            .topics(["agent.conversation.s1".into()].into())
            .await
            .unwrap();
        assert_eq!(
            next(&mut events).await.data.unwrap(),
            json!({"session_id":"s1","ready":true})
        );
        assert_eq!(
            next(&mut events).await.data.unwrap(),
            serde_json::to_value(initial).unwrap()
        );
        client
            .topics(
                [
                    "agent.conversation.s1".into(),
                    "agent.conversation.s2".into(),
                ]
                .into(),
            )
            .await
            .unwrap();
        assert_eq!(next(&mut events).await.data.unwrap()["session_id"], "s2");
        assert_eq!(source.starts.load(Ordering::SeqCst), 1);
        assert_eq!(source.delta.receiver_count(), 1);
        client
            .topics(["agent.conversation.s2".into()].into())
            .await
            .unwrap();
        until(|| source.delta.receiver_count() == 1).await;
        source.delta.send(delta("s1", 2, "not wanted")).unwrap();
        source.delta.send(delta("s2", 2, "wanted")).unwrap();
        assert_eq!(next(&mut events).await.data.unwrap()["session_id"], "s2");
        client.topics(Default::default()).await.unwrap();
        until(|| source.delta.receiver_count() == 0).await;
        drop(client);
        tokio::task::spawn_blocking(move || hub.shutdown())
            .await
            .unwrap()
            .unwrap();
    }
    #[tokio::test]
    async fn broker_rejects_stale_generation_and_hidden_or_unknown_payloads() {
        let source = FixtureSource::new(8, vec![]);
        let (hub, service, client, mut events) = rig(source.clone(), shown(), None).await;
        client
            .topics(["agent.conversation.s1".into()].into())
            .await
            .unwrap();
        next(&mut events).await;
        let old = service.state.lock().unwrap().wanted["s1"].generation;
        client.topics(Default::default()).await.unwrap();
        client
            .topics(["agent.conversation.s1".into()].into())
            .await
            .unwrap();
        assert_eq!(next(&mut events).await.data.unwrap()["ready"], true);
        hub.handle()
            .publish_live_stream(Delivery {
                session: "s1".into(),
                generation: old,
                kind: Kind::Delta,
                data: serde_json::to_value(delta("s1", 99, "stale")).unwrap(),
            })
            .await
            .unwrap();
        source.delta.send(delta("s1", 100, "fresh")).unwrap();
        let event = next(&mut events).await;
        assert_eq!(event.data.unwrap()["seq"], 100);
        service.rows.write().unwrap().remove("s1");
        source.delta.send(delta("s1", 101, "dismissed")).unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(80), events.recv())
                .await
                .is_err()
        );
        drop(client);
        tokio::task::spawn_blocking(move || hub.shutdown())
            .await
            .unwrap()
            .unwrap();
    }
    #[tokio::test]
    async fn status_lines_merge_known_rows_without_disclosing_hidden_or_unknown_ids() {
        let source = FixtureSource::new(8, vec![]);
        let mut rows = shown();
        rows.insert(
            "hidden".into(),
            json!({"session_id":"hidden","mode":"stopped","updated_at":"2020-01-01T00:00:00Z"}),
        );
        let (hub, service, client, mut events) = rig(source.clone(), rows, None).await;
        client
            .topics(["agent.statusline".into()].into())
            .await
            .unwrap();
        let status = StatusLine {
            model_display: Some("fixture-model".into()),
            cost_usd: Some(41.72),
            ..Default::default()
        };
        let expected = serde_json::to_value(&status).unwrap();
        for id in ["unknown", "hidden", "s1"] {
            source
                .status
                .send(StatusLineUpdate {
                    session_id: id.into(),
                    cwd: None,
                    status_line: status.clone(),
                })
                .unwrap();
        }
        let event = next(&mut events).await;
        assert_eq!(event.topic, "agent.statusline");
        assert_eq!(
            event.data.unwrap(),
            json!({"sessionId":"s1","statusLine":expected})
        );
        let rows = service.rows.read().unwrap().clone();
        assert!(!rows.contains_key("unknown"));
        assert_eq!(rows["s1"]["status_line"]["cost_usd"], 41.72);
        assert_eq!(rows["s1"]["statusLine"]["modelDisplay"], "fixture-model");
        assert_eq!(rows["hidden"]["mode"], "stopped");
        assert_eq!(source.delta.receiver_count(), 0);
        assert!(events.try_recv().is_err());
        drop(client);
        tokio::task::spawn_blocking(move || hub.shutdown())
            .await
            .unwrap()
            .unwrap();
    }
    #[tokio::test]
    async fn lag_requires_new_ready_without_replaying_buffered_fragments() {
        let source = FixtureSource::new(
            1,
            vec![
                delta("s1", 1, "old-one"),
                delta("s1", 2, "old-two"),
                delta("s1", 3, "old-three"),
            ],
        );
        let (hub, _, client, mut events) = rig(source.clone(), shown(), None).await;
        client
            .topics(["agent.conversation.s1".into()].into())
            .await
            .unwrap();
        until(|| source.starts.load(Ordering::SeqCst) >= 2).await;
        source.delta.send(delta("s1", 100, "after gap")).unwrap();
        let mut ready = false;
        loop {
            let data = next(&mut events).await.data.unwrap();
            if data["ready"] == true {
                ready = true;
                continue;
            }
            assert!(ready, "delta preceded resync boundary");
            assert_eq!(data["seq"], 100);
            assert_eq!(data["reset"], false);
            break;
        }
        drop(client);
        tokio::task::spawn_blocking(move || hub.shutdown())
            .await
            .unwrap()
            .unwrap();
    }
    #[tokio::test]
    async fn upstream_layout_curates_hidden_history_and_new_visibility_gets_ready() {
        let source = FixtureSource::new(8, vec![]);
        let upstream = Arc::new(RwLock::new(json!({"version":1,"data":{"agents":[]}})));
        let rows = BTreeMap::from([(
            "s1".into(),
            json!({"session_id":"s1","mode":"stopped","updated_at":"2026-09-28T00:00:00Z"}),
        )]);
        let (hub, _, client, mut events) = rig(source.clone(), rows, Some(upstream.clone())).await;
        client
            .topics(
                [
                    "agent.conversation.s1".into(),
                    "agent.conversation.unknown".into(),
                ]
                .into(),
            )
            .await
            .unwrap();
        until(|| source.delta.receiver_count() == 1).await;
        source.delta.send(delta("s1", 1, "hidden")).unwrap();
        source.delta.send(delta("unknown", 1, "unknown")).unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(80), events.recv())
                .await
                .is_err()
        );
        *upstream.write().unwrap() = json!({"version":2,"data":{"agents":[{"sessionId":"s1"}]}});
        let ready = next(&mut events).await;
        assert_eq!(ready.data.unwrap(), json!({"session_id":"s1","ready":true}));
        source.delta.send(delta("s1", 2, "shown")).unwrap();
        assert_eq!(next(&mut events).await.data.unwrap()["seq"], 2);
        drop(client);
        tokio::task::spawn_blocking(move || hub.shutdown())
            .await
            .unwrap()
            .unwrap();
    }
}
