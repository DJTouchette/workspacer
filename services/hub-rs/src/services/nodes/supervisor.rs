use super::{
    cloud::{self, Cloud},
    model::*,
};
use crate::services::agent_lifecycle::Operation;
use anyhow::{Result, anyhow, bail};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    sync::{mpsc, oneshot, watch},
    time::Instant,
};
#[derive(Clone, Debug)]
pub enum Reading {
    Answered {
        connection: u64,
        node: String,
        last_exit: Option<ExitRecord>,
    },
    Absent,
    Silent(u64),
    Unavailable,
}
pub trait Probe: Send + Sync + 'static {
    fn read(&self, timeout: Duration) -> Operation<'_, Reading>;
    fn evict(&self, connection: u64) -> Operation<'_, bool>;
    fn close(&self) {}
}
#[derive(Clone, Debug)]
pub struct Change {
    pub node: View,
    pub previous: State,
}
pub type Publish = Arc<dyn Fn(Change) + Send + Sync>;
#[derive(Clone, Copy, PartialEq)]
enum Direction {
    Wake,
    Sleep,
}
struct Memory {
    view: View,
    epoch: u64,
    direction: Option<Direction>,
    deadline: Option<Instant>,
}
struct Cell {
    memory: Mutex<Memory>,
    epoch: watch::Sender<u64>,
    action: tokio::sync::Mutex<()>,
}
struct Request {
    id: String,
    epoch: u64,
    direction: Direction,
    reply: oneshot::Sender<Result<View>>,
    prior_power: bool,
    change: Change,
}
pub struct Supervisor {
    order: Vec<String>,
    cells: BTreeMap<String, Arc<Cell>>,
    clients: BTreeMap<String, Arc<dyn Cloud>>,
    probe: Arc<dyn Probe>,
    timings: Timings,
    publish: Publish,
    queue: mpsc::Sender<Request>,
    receiver: Mutex<Option<mpsc::Receiver<Request>>>,
    stop: watch::Sender<bool>,
    strikes: Mutex<(Option<u64>, u32)>,
}
impl Supervisor {
    pub fn new(
        nodes: Vec<Node>,
        clients: BTreeMap<String, Arc<dyn Cloud>>,
        probe: Arc<dyn Probe>,
        timings: Timings,
        publish: Publish,
    ) -> Result<Arc<Self>> {
        anyhow::ensure!(
            nodes.len() <= 256
                && !timings.poll.is_zero()
                && !timings.probe.is_zero()
                && !timings.register.is_zero()
                && !timings.register_poll.is_zero()
                && !timings.stop_timeout.is_zero()
                && timings.start_retries <= 3,
            "invalid node supervisor limits"
        );
        let mut cells = BTreeMap::new();
        let mut order = vec![];
        for node in nodes {
            anyhow::ensure!(
                valid_id(&node.id) && !cells.contains_key(&node.id),
                "invalid or duplicate node id"
            );
            let id = node.id.clone();
            let view=View{id:id.clone(),label:node.label().into(),state:State::Unreachable,since:now(),last_seen:0,detail:"not reconciled yet — the hub has just started and has not asked the cloud API anything".into(),wakeable:node.coordinates()&&clients.contains_key(&id),last_exit:None,slept_by_hub:false,may_be_running:false,wake_failures:0};
            let (epoch, _) = watch::channel(0);
            cells.insert(
                id.clone(),
                Arc::new(Cell {
                    memory: Mutex::new(Memory {
                        view,
                        epoch: 0,
                        direction: None,
                        deadline: None,
                    }),
                    epoch,
                    action: tokio::sync::Mutex::new(()),
                }),
            );
            order.push(id);
        }
        let (queue, receiver) = mpsc::channel(64);
        let (stop, _) = watch::channel(false);
        Ok(Arc::new(Self {
            order,
            cells,
            clients,
            probe,
            timings,
            publish,
            queue,
            receiver: Mutex::new(Some(receiver)),
            stop,
            strikes: Mutex::new((None, 0)),
        }))
    }
    pub fn list(&self) -> Vec<View> {
        self.order
            .iter()
            .map(|id| self.cells[id].memory.lock().unwrap().view.clone())
            .collect()
    }
    pub fn view(&self, id: &str) -> Result<View> {
        Ok(self
            .cells
            .get(id)
            .ok_or_else(|| anyhow!("unknown node"))?
            .memory
            .lock()
            .unwrap()
            .view
            .clone())
    }
    fn set(
        &self,
        id: &str,
        epoch: Option<u64>,
        next: State,
        detail: String,
        power: Option<bool>,
        change: impl FnOnce(&mut Memory),
    ) -> bool {
        let cell = &self.cells[id];
        let event = {
            let mut memory = cell.memory.lock().unwrap();
            if epoch.is_some_and(|epoch| epoch != memory.epoch) {
                return false;
            }
            let old = memory.view.clone();
            change(&mut memory);
            if memory.view.state != next {
                memory.view.since = now()
            }
            memory.view.state = next;
            memory.view.detail = detail;
            if let Some(power) = power {
                memory.view.may_be_running = power
            }
            if old.state == memory.view.state
                && old.detail == memory.view.detail
                && old.may_be_running == memory.view.may_be_running
                && old.last_exit == memory.view.last_exit
                && old.wake_failures == memory.view.wake_failures
                && old.slept_by_hub == memory.view.slept_by_hub
            {
                None
            } else {
                Some(Change {
                    node: memory.view.clone(),
                    previous: old.state,
                })
            }
        };
        if let Some(event) = event {
            (self.publish)(event)
        }
        true
    }
    pub async fn wake(&self, id: &str) -> Result<View> {
        self.request(id, Direction::Wake).await
    }
    pub async fn sleep(&self, id: &str) -> Result<View> {
        self.request(id, Direction::Sleep).await
    }
    async fn request(&self, id: &str, direction: Direction) -> Result<View> {
        anyhow::ensure!(!*self.stop.borrow(), "node supervisor is closing");
        let cell = self.cells.get(id).ok_or_else(|| anyhow!("unknown node"))?;
        let reply = {
            let mut memory = cell.memory.lock().unwrap();
            if memory.direction == Some(direction)
                || (direction == Direction::Wake && memory.view.state == State::Available)
                || (direction == Direction::Sleep && memory.view.state == State::Stopped)
            {
                return Ok(memory.view.clone());
            }
            anyhow::ensure!(
                memory.view.wakeable,
                "node has no cloud coordinates or credential on this hub"
            );
            let permit = self
                .queue
                .try_reserve()
                .map_err(|_| anyhow!("node action queue is full or closed"))?;
            memory.epoch = memory.epoch.wrapping_add(1);
            memory.direction = Some(direction);
            memory.deadline = Some(
                Instant::now()
                    + if direction == Direction::Wake {
                        self.timings.register
                    } else {
                        self.timings.stop_timeout
                    },
            );
            let prior_power = memory.view.may_be_running;
            let old = memory.view.state;
            memory.view.state = if direction == Direction::Wake {
                State::Waking
            } else {
                State::Stopping
            };
            memory.view.since = now();
            memory.view.may_be_running = true;
            memory.view.detail = if direction == Direction::Wake {
                "starting the machine"
            } else {
                "asking the machine to shut down cleanly"
            }
            .into();
            if direction == Direction::Wake {
                memory.view.slept_by_hub = false
            }
            cell.epoch.send_replace(memory.epoch);
            let (tx, rx) = oneshot::channel();
            let change = Change {
                node: memory.view.clone(),
                previous: old,
            };
            let request = Request {
                id: id.into(),
                epoch: memory.epoch,
                direction,
                reply: tx,
                prior_power,
                change,
            };
            permit.send(request);
            rx
        };
        reply
            .await
            .map_err(|_| anyhow!("node supervisor stopped; cloud action outcome may be unknown"))?
    }
    fn current(&self, id: &str, epoch: u64) -> bool {
        !*self.stop.borrow() && self.cells[id].memory.lock().unwrap().epoch == epoch
    }
    async fn superseded(&self, id: &str, epoch: u64) {
        let mut changed = self.cells[id].epoch.subscribe();
        let mut stop = self.stop.subscribe();
        loop {
            if *changed.borrow() != epoch || *stop.borrow() {
                return;
            }
            tokio::select! {_=changed.changed()=>{},_=stop.changed()=>{}}
        }
    }
    fn attributed(&self, named: &str) -> Option<String> {
        if self.cells.contains_key(named) {
            Some(named.into())
        } else if self.order.len() == 1 {
            Some(self.order[0].clone())
        } else {
            None
        }
    }
    async fn reading(&self) -> Reading {
        match tokio::time::timeout(
            self.timings.probe + Duration::from_millis(100),
            self.probe.read(self.timings.probe),
        )
        .await
        {
            Ok(Ok(reading)) => reading,
            _ => Reading::Unavailable,
        }
    }
    fn available(&self, id: &str, epoch: Option<u64>, exit: Option<ExitRecord>) {
        let detail = exit
            .as_ref()
            .filter(|exit| !exit.clean())
            .map(ExitRecord::describe)
            .unwrap_or_default();
        self.set(id, epoch, State::Available, detail, Some(true), |memory| {
            memory.direction = None;
            memory.deadline = None;
            memory.view.last_seen = now();
            memory.view.wake_failures = 0;
            if exit.is_some() {
                memory.view.last_exit = exit;
            }
        });
    }
    pub async fn reconcile(&self) {
        if *self.stop.borrow() {
            return;
        }
        let reading = self.reading().await;
        if matches!(reading, Reading::Unavailable) {
            return;
        }
        let attached = match &reading {
            Reading::Answered { node, .. } => self.attributed(node),
            _ => None,
        };
        let evict = {
            let mut strikes = self.strikes.lock().unwrap();
            match reading {
                Reading::Silent(connection) => {
                    if strikes.0 != Some(connection) {
                        *strikes = (Some(connection), 0)
                    }
                    strikes.1 += 1;
                    if strikes.1 >= self.timings.silent_strikes.max(1) {
                        *strikes = (None, 0);
                        Some(connection)
                    } else {
                        None
                    }
                }
                _ => {
                    *strikes = (None, 0);
                    None
                }
            }
        };
        if let Some(connection) = evict {
            let _ = self.probe.evict(connection).await;
        }
        use futures_util::StreamExt;
        futures_util::stream::iter(self.order.clone()).map(|owned_id|{let attached=attached.clone();let reading=reading.clone();async move{let id=&owned_id;
   let epoch={let memory=self.cells[id].memory.lock().unwrap();if memory.direction.is_some(){return}memory.epoch};if attached.as_ref()==Some(id){let exit=match reading{Reading::Answered{last_exit,..}=>last_exit,_=>None};self.available(id,Some(epoch),exit);return}
   let Some(cloud)=self.clients.get(id)else{self.set(id,Some(epoch),State::Unreachable,"its provider is not on the bus, and the hub holds no cloud credentials for this node, so it cannot tell a sleeping machine from a broken one".into(),None,|_|{});return};
   let result=tokio::time::timeout(Duration::from_secs(30),cloud.state()).await.unwrap_or(Err(cloud::Failure::Timeout));if !self.current(id,epoch){return}match result{
    Err(error)=>{self.set(id,Some(epoch),State::Unreachable,format!("could not read the machine's state — {error}"),None,|_|{});},
    Ok(cloud::State::Started)=>{self.set(id,Some(epoch),State::Unreachable,"the machine is running but its provider has not registered with the hub — check its upstream connection and boot log".into(),Some(true),|_|{});},
    Ok(cloud::State::Starting|cloud::State::Replacing)=>{self.set(id,Some(epoch),State::Waking,"the machine is starting".into(),Some(true),|_|{});},
    Ok(cloud::State::Stopped|cloud::State::Suspended)=>{let view=self.view(id).unwrap();let failed=view.wake_failures>0;let detail=if failed{"the machine is stopped, but recent wake attempts did not register a provider; it may be failing on boot"}else if view.slept_by_hub{"this hub put it to sleep; nothing is billing"}else{""};self.set(id,Some(epoch),if failed{State::Unreachable}else{State::Stopped},detail.into(),Some(false),|memory|memory.view.last_exit=None);},
    Ok(cloud::State::Destroyed)=>{self.set(id,Some(epoch),State::Unreachable,"the machine has been destroyed".into(),Some(false),|_|{});},
    Ok(cloud::State::Unknown)=>{self.set(id,Some(epoch),State::Unreachable,"the cloud API reports an unfamiliar state".into(),None,|_|{});}
   }
  }}).buffer_unordered(8).collect::<Vec<_>>().await;
    }
    async fn action(self: Arc<Self>, request: Request) {
        let Request {
            id,
            epoch,
            direction,
            reply,
            prior_power,
            ..
        } = request;
        let cell = &self.cells[&id];
        let cloud = self.clients[&id].clone();
        let lock = cell.action.lock().await;
        if !self.current(&id, epoch) {
            let _ = reply.send(self.view(&id));
            return;
        }
        if direction == Direction::Wake {
            match self.reading().await {
                Reading::Answered {
                    node, last_exit, ..
                } if self.attributed(&node).as_deref() == Some(&id) => {
                    self.available(&id, Some(epoch), last_exit);
                    let _ = reply.send(self.view(&id));
                    return;
                }
                Reading::Silent(connection) => {
                    let _ = self.probe.evict(connection).await;
                }
                _ => {}
            }
            let mut failure = None;
            for attempt in 0..=self.timings.start_retries {
                if !self.current(&id, epoch) {
                    let _ = reply.send(self.view(&id));
                    return;
                }
                if attempt > 0 {
                    tokio::select! {_=self.superseded(&id,epoch)=>{let _=reply.send(self.view(&id));return},_=tokio::time::sleep(self.timings.retry_delay)=>{}}
                }
                match tokio::time::timeout(Duration::from_secs(30), cloud.start())
                    .await
                    .unwrap_or(Err(cloud::Failure::Timeout))
                {
                    Ok(()) => {
                        failure = None;
                        break;
                    }
                    Err(error) => {
                        failure = Some(error);
                        if matches!(
                            error,
                            cloud::Failure::Status(404) | cloud::Failure::RateLimited
                        ) {
                            break;
                        }
                    }
                }
            }
            if let Some(error) = failure {
                let power = match error {
                    cloud::Failure::Status(404) => Some(false),
                    cloud::Failure::Status(status) if (400..500).contains(&status) => {
                        Some(prior_power)
                    }
                    cloud::Failure::RateLimited => Some(prior_power),
                    _ => None,
                };
                self.set(
                    &id,
                    Some(epoch),
                    State::Unreachable,
                    format!("the machine start was not confirmed — {error}"),
                    power,
                    |memory| {
                        memory.direction = None;
                        memory.view.wake_failures += 1
                    },
                );
                let _ = reply.send(Err(error.into()));
                return;
            }
            let _ = reply.send(self.view(&id));
            drop(lock);
            self.watch_wake(&id, epoch, cloud).await;
        } else {
            let result =
                tokio::time::timeout(Duration::from_secs(30), cloud.stop(self.timings.stop_grace))
                    .await
                    .unwrap_or(Err(cloud::Failure::Timeout));
            if let Err(error) = result {
                self.set(&id,Some(epoch),State::Unreachable,format!("the cloud API did not confirm the stop — {error}; it may still be running and billing"),Some(true),|memory|memory.direction=None);
                let _ = reply.send(Err(error.into()));
                return;
            }
            let _ = reply.send(self.view(&id));
            drop(lock);
            self.watch_sleep(&id, epoch, cloud, false).await;
        }
    }
    async fn watch_wake(&self, id: &str, epoch: u64, cloud: Arc<dyn Cloud>) {
        let deadline = self.cells[id]
            .memory
            .lock()
            .unwrap()
            .deadline
            .unwrap_or(Instant::now() + self.timings.register);
        let waited = tokio::select! {_=self.superseded(id,epoch)=>return,result=tokio::time::timeout_at(deadline,cloud.wait(cloud::State::Started,self.timings.register))=>result};
        if matches!(waited, Ok(Ok(()))) {
            self.set(
                id,
                Some(epoch),
                State::Waking,
                "the machine is up; waiting for its provider to register".into(),
                Some(true),
                |_| {},
            );
        }
        loop {
            if !self.current(id, epoch) {
                return;
            }
            let reading = tokio::select! {_=self.superseded(id,epoch)=>return,result=tokio::time::timeout_at(deadline,self.reading())=>result};
            if let Ok(Reading::Answered {
                node, last_exit, ..
            }) = reading
            {
                if self.attributed(&node).as_deref() == Some(id) {
                    self.available(id, Some(epoch), last_exit);
                    return;
                }
            }
            if Instant::now() >= deadline {
                break;
            }
            tokio::select! {_=self.superseded(id,epoch)=>return,_=tokio::time::sleep_until((Instant::now()+self.timings.register_poll).min(deadline))=>{}}
        }
        if !self.set(
            id,
            Some(epoch),
            State::Unreachable,
            "the machine was started but its provider did not register; check the node boot log"
                .into(),
            Some(true),
            |memory| {
                memory.direction = None;
                memory.view.wake_failures += 1
            },
        ) {
            return;
        }
        if self.timings.keep_failed_wakes_running {
            return;
        }
        let _lock = self.cells[id].action.lock().await;
        if !self.current(id, epoch) {
            return;
        }
        // A fresh live answer wins even at the deadline. Failure of our own probe
        // cannot justify destroying work on a machine whose state we could not read.
        match self.reading().await {
            Reading::Answered {
                node, last_exit, ..
            } if self.attributed(&node).as_deref() == Some(id) => {
                self.available(id, Some(epoch), last_exit);
                return;
            }
            Reading::Unavailable => return,
            _ => {}
        }
        if !self.current(id, epoch)
            || self
                .view(id)
                .is_ok_and(|view| view.state == State::Available)
        {
            return;
        }
        let stopped =
            tokio::time::timeout(Duration::from_secs(30), cloud.stop(self.timings.stop_grace))
                .await
                .unwrap_or(Err(cloud::Failure::Timeout));
        if let Err(error) = stopped {
            self.set(id,Some(epoch),State::Unreachable,format!("the failed wake could not be stopped — {error}; it may still be running and billing"),Some(true),|_|{});
            return;
        }
        self.watch_sleep(id, epoch, cloud, true).await;
    }
    async fn watch_sleep(&self, id: &str, epoch: u64, cloud: Arc<dyn Cloud>, failed_wake: bool) {
        let result = tokio::select! {_=self.superseded(id,epoch)=>return,result=tokio::time::timeout(self.timings.stop_timeout,cloud.wait(cloud::State::Stopped,self.timings.stop_timeout))=>result};
        if matches!(result, Ok(Ok(()))) {
            self.set(id,Some(epoch),if failed_wake{State::Unreachable}else{State::Stopped},if failed_wake{"the provider did not register, so the hub stopped the machine again rather than leave it billing".into()}else{String::new()},Some(false),|memory|{memory.direction=None;memory.view.last_exit=None;if !failed_wake{memory.view.slept_by_hub=true;}});
        } else {
            self.set(id,Some(epoch),State::Unreachable,"the machine was asked to shut down but has not reported stopped; it may still be running and billing".into(),Some(true),|memory|memory.direction=None);
        }
    }
    pub fn close(&self) {
        self.stop.send_replace(true);
        self.probe.close();
    }
    pub async fn run(self: Arc<Self>) -> Result<()> {
        let mut receiver = self
            .receiver
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| anyhow!("node supervisor already running"))?;
        let mut stop = self.stop.subscribe();
        let mut tick = tokio::time::interval(self.timings.poll);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut tasks = tokio::task::JoinSet::new();
        let mut reconciling = false;
        loop {
            if *stop.borrow() {
                receiver.close();
                tasks.abort_all();
                while tasks.join_next().await.is_some() {}
                return Ok(());
            }
            tokio::select! {biased;
             _=stop.changed()=>{receiver.close();tasks.abort_all();while tasks.join_next().await.is_some(){}return Ok(())}
             Some(done)=tasks.join_next()=>{match done{Ok(true)=>reconciling=false,Ok(false)=>{},Err(error)=>bail!("node supervisor task failed: {error}")}}
             request=receiver.recv()=>{let Some(request)=request else{return Ok(())};(self.publish)(request.change.clone());let service=self.clone();tasks.spawn(async move{service.action(request).await;false});}
             _=tick.tick(),if !reconciling=>{reconciling=true;let service=self.clone();tasks.spawn(async move{service.reconcile().await;true});}
            }
        }
    }
}
fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
