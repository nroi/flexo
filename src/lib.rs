use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::thread;
use std::thread::JoinHandle;
use std::sync::mpsc::{channel, Sender, Receiver};
use crate::ChannelEstablishment::{NewChannel, ExistingChannel};
use std::collections::hash_map::Entry;

const NUM_MAX_ATTEMPTS: i32 = 100;

#[derive(Debug)]
pub struct JobPartiallyCompleted<C> {
    pub channel: C,
    pub continue_at: u64
}

#[derive(Debug)]
pub struct JobTerminated<C, E> {
    pub channel: C,
    pub error: E,
}

impl <C> JobPartiallyCompleted<C> {
    pub fn new(channel: C, continue_at: u64) -> Self {
        Self {
            channel,
            continue_at
        }
    }
}

#[derive(Debug)]
pub struct JobCompleted<C, P> {
    pub channel: C,
    pub provider: P
}

impl <C, P> JobCompleted<C, P> {
    pub fn new(channel: C, provider: P) -> Self {
        Self {
            channel,
            provider,
        }
    }
}

#[derive(Debug)]
pub enum JobResult<J> where J: Job {
    Complete(JobCompleted<J::C, J::P>),
    Partial(JobPartiallyCompleted<J::C>),
    Error(JobTerminated<J::C, J::E>),
}

#[derive(PartialEq, Eq, Clone, Debug)]
// TODO don't use CS as generic type: Use only the Job as the generic type, the job should already include all
// required types.
pub enum JobOutcome <P, CS> where
    P: std::cmp::Eq + std::hash::Hash,
    CS: ChannelState + std::marker::Copy,
{
    Success(P, CS),
    Error(HashMap<P, i32>, CS),
}

impl <J> JobResult<J> where J: Job {
    fn is_success(&self) -> bool {
        match self {
            JobResult::Complete(_) => true,
            _ => false,
        }
    }
}

pub trait Provider where
    Self: std::marker::Sized + std::fmt::Debug + std::clone::Clone + std::cmp::Eq + std::hash::Hash + std::marker::Send + 'static,
{
    type J: Job;
    fn new_job(&self, order: <<Self as Provider>::J as Job>::O) -> Self::J;

    /// returns an identifier that remains unchanged throughout the lifetime of the program.
    /// the intention is that while some properties of the provider change (i.e., its score),
    /// we still need to be able to recognize: Although those two Providers are not equal (p1 != p2),
    /// they actually refer to the same provider (p1.identity() = p2.identity()).
    fn identifier(&self) -> &<<Self as Provider>::J as Job>::PI;
    fn score(&self) -> <<Self as Provider>::J as Job>::S;
}

pub trait Job where
    Self: std::marker::Sized + std::fmt::Debug + std::marker::Send,
{
    type S: std::cmp::Ord;
    type JS: JobState;
    type C: Channel<J=Self>;
    type O: Order<J=Self> + std::clone::Clone + std::cmp::Eq + std::hash::Hash;
    type P: Provider<J=Self>;
    type E: std::fmt::Debug;
    type CS: ChannelState<J=Self> + std::marker::Copy;
    type PI: std::cmp::Eq;

    fn provider(&self) -> &Self::P;
    fn order(&self) -> Self::O;
    fn execute(self, channel: Self::C) -> JobResult<Self>;
    fn get_channel(&self, channels: &Arc<Mutex<HashMap<Self::P, Self::C>>>) -> (Self::C, ChannelEstablishment) {
        let mut channels = channels.lock().unwrap();
        match channels.remove(&self.provider()) {
            Some(mut channel) => {
                println!("Reusing previous channel: {:?}", &self.provider());
                channel.reset_order(self.order());
                (channel, ExistingChannel)
            }
            None => {
                println!("need to create new channel: {:?}", &self.provider());
                let channel = self.order().new_channel();
                (channel, NewChannel)
            }
        }
    }
}

pub trait Order where Self: std::marker::Sized + std::clone::Clone + std::cmp::Eq + std::hash::Hash + std::fmt::Debug + std::marker::Send + 'static {
    type J: Job<O=Self>;
    fn new_channel(self) -> <<Self as Order>::J as Job>::C;

    fn try_until_success(
        self,
        mut providers: &mut Vec<<<Self as Order>::J as Job>::P>,
        provider_failures: &mut Arc<Mutex<HashMap<<<Self as Order>::J as Job>::P , i32>>>,
        provider_current_usages: &mut Arc<Mutex<HashMap<<<Self as Order>::J as Job>::P, i32>>>,
        channels: Arc<Mutex<HashMap<<<Self as Order>::J as Job>::P, <<Self as Order>::J as Job>::C>>>,
        tx: Sender<FlexoMessage<<<Self as Order>::J as Job>::P>>
    ) -> JobResult<Self::J> {
        let mut num_attempt = 0;
        let mut punished_providers = Vec::new();
        let result = loop {
            num_attempt += 1;
            println!("available providers: {:?}", &providers);
            let provider = self.select_provider(&mut providers, provider_current_usages.lock().unwrap(), provider_failures.lock().unwrap());
            let message = FlexoMessage::ProviderSelected(provider.clone());
            let _ = tx.send(message);
            {
                let mut provider_current_usages = provider_current_usages.lock().unwrap();
                let value = provider_current_usages.entry(provider.clone()).or_insert(0);
                *value += 1;
            }
            println!("selected provider: {:?}", &provider);
            let self_cloned: Self = self.clone();
            let job = provider.new_job(self_cloned);
            let (channel, channel_establishment) = job.get_channel(&channels);
            let _ = tx.send(FlexoMessage::ChannelEstablished(channel_establishment));
            let result = job.execute(channel);
            match &result {
                JobResult::Complete(_) => {
                    reward(provider.clone(), provider_failures.lock().unwrap());
                },
                JobResult::Partial(partial_job) => {
                    punish(provider.clone(), provider_failures.lock().unwrap());
                    punished_providers.push(provider.clone());
                    println!("will continue job at {:?}", partial_job.continue_at);
                },
                JobResult::Error(e) => {
                    punish(provider.clone(), provider_failures.lock().unwrap());
                    punished_providers.push(provider.clone());
                    println!("Error: {:?}, try again", e)
                },
            };
            if result.is_success() || providers.is_empty() || num_attempt >= NUM_MAX_ATTEMPTS {
                break result;
            }
        };
        if !result.is_success() {
            pardon(punished_providers, provider_failures.lock().unwrap());
        }

        result
    }

    fn select_provider(
        &self,
        providers: &mut Vec<<<Self as Order>::J as Job>::P>,
        provider_current_usages: MutexGuard<HashMap<<<Self as Order>::J as Job>::P, i32>>,
        provider_failures: MutexGuard<HashMap<<<Self as Order>::J as Job>::P, i32>>,
    ) -> <<Self as Order>::J as Job>::P {
        let (idx, (_, _, _, _)) = providers
            .iter()
            .map(|x| (provider_failures.get(&x).unwrap_or(&0), provider_current_usages.get(&x).unwrap_or(&0), x.score(), x))
            .enumerate()
            .min_by(|(_idx_x, (num_failures_x, num_usages_x, score_x, _x)),
                     (_idx_y, (num_failures_y, num_usages_y, score_y, _y))|
                (num_failures_x, num_usages_x, score_x).cmp(&(num_failures_y, num_usages_y, score_y)))
            .unwrap();

        providers.remove(idx)
    }
}

pub trait Channel where
    Self: std::marker::Sized + std::fmt::Debug + std::marker::Send + 'static,
{
    type J: Job;

    fn progress_indicator(&self) -> Option<u64>;
    fn reset_order(&mut self, order: <<Self as Channel>::J as Job>::O);
    fn channel_state_item(&mut self) -> &mut JobStateItem<Self::J>;
    fn channel_state(&self) -> <<Self as Channel>::J as Job>::CS;
    fn channel_state_ref(&mut self) -> &mut <<Self as Channel>::J as Job>::CS;
    fn reset(&mut self) {
        self.channel_state_item().reset();
        self.channel_state_ref().reset();
    }
}

pub trait ChannelState where Self: std::marker::Send + 'static {
    type J: Job;
    fn reset(&mut self);
}

#[derive(PartialEq, Eq, Hash, Clone, Debug, Copy)]
pub enum ChannelEstablishment {
    NewChannel,
    ExistingChannel,
}

/// Marker trait.
pub trait JobState {}

#[derive(Debug)]
pub struct JobStateItem<J> where J: Job
{
    pub order: J::O,
    // Used to manage the resources acquired for a job. It is set to Some(_) if this there is an active job associated
    // with the Channel, or None if the channel is just kept open for requests that may arrive in the future. The
    // reason for using Optional (rather than just JS) is that this way, drop() will called on the JS as soon as we
    // reset the state to None, so that acquired resources are released as soon as possible.
    pub state: Option<J::JS>
}

impl <J> JobStateItem<J> where J: Job {
    fn reset(&mut self) {
        self.state = None
    }
}



fn punish<P>(provider: P, mut failures: MutexGuard<HashMap<P, i32>>) where
    P: std::cmp::Eq + std::hash::Hash + std::clone::Clone + std::fmt::Debug,
{
    let value = failures.entry(provider).or_insert(0);
    *value += 1;
}

fn reward<P>(provider: P, mut failures: MutexGuard<HashMap<P, i32>>) where
    P: std::cmp::Eq + std::hash::Hash + std::clone::Clone + std::fmt::Debug,
{
    let value = failures.entry(provider).or_insert(0);
    *value -= 1;
}

fn pardon<P>(punished_providers: Vec<P>, mut failures: MutexGuard<HashMap<P, i32>>) where
    P: std::cmp::Eq + std::hash::Hash + std::clone::Clone + std::fmt::Debug,
{
    for not_guilty in punished_providers {
        match (*failures).entry(not_guilty.clone()) {
            Entry::Occupied(mut value) => {
                let value = value.get_mut();
                *value -= 1;
            },
            Entry::Vacant(_) => {},
        }
    }
}

/// The context in which a job is executed, including all stateful information required by the job.
/// This context is meant to be initialized once during the program's lifecycle.
pub struct JobContext<J> where
    J: Job,
{
    providers: Arc<Mutex<Vec<J::P>>>,
    channels: Arc<Mutex<HashMap<J::P, J::C>>>,
    orders_in_progress: Arc<Mutex<HashSet<J::O>>>,
    providers_in_use: Arc<Mutex<HashMap<J::P, i32>>>,
    panic_monitor: Vec<Arc<Mutex<i32>>>,
    provider_failures: Arc<Mutex<HashMap<J::P, i32>>>,
}

pub struct ScheduledItem<P, CS> where
    P: std::cmp::Eq + std::hash::Hash,
    CS: ChannelState + std::marker::Copy,
{
pub join_handle: JoinHandle<JobOutcome<P, CS>>,
    pub rx: Receiver<FlexoMessage<P>>,
}

pub enum ScheduleOutcome <P, CS> where
    P: std::cmp::Eq + std::hash::Hash,
    CS: ChannelState + std::marker::Copy,
{
    Skipped,
    Scheduled(ScheduledItem<P, CS>),
}

#[derive(PartialEq, Eq, Hash, Clone, Debug)]
pub enum FlexoMessage <P> {
    ProviderSelected(P),
    ChannelEstablished(ChannelEstablishment),
}

impl <J> JobContext<J> where J: Job {
    pub fn new(initial_providers: Vec<J::P>) -> Self {
        let providers: Arc<Mutex<Vec<J::P>>> = Arc::new(Mutex::new(initial_providers));
        let channels: Arc<Mutex<HashMap<J::P, J::C>>> = Arc::new(Mutex::new(HashMap::new()));
        let orders_in_progress: Arc<Mutex<HashSet<J::O>>> = Arc::new(Mutex::new(HashSet::new()));
        let providers_in_use: Arc<Mutex<HashMap<J::P, i32>>> = Arc::new(Mutex::new(HashMap::new()));
        let provider_records: Arc<Mutex<HashMap<J::P, i32>>> = Arc::new(Mutex::new(HashMap::new()));
        let thread_mutexes: Vec<Arc<Mutex<i32>>> = Vec::new();
        Self {
            providers,
            channels,
            orders_in_progress,
            provider_failures: provider_records,
            providers_in_use,
            panic_monitor: thread_mutexes,
        }
    }

    pub fn schedule(&mut self, order: J::O) -> ScheduleOutcome<J::P, J::CS> {
        let mutex = Arc::new(Mutex::new(0));
        let mutex_cloned = Arc::clone(&mutex);
        self.panic_monitor = self.panic_monitor.drain(..).filter(|mutex| {
            match mutex.try_lock() {
                Ok(_) => {
                    false
                },
                Err(TryLockError::WouldBlock) => {
                    true
                },
                Err(TryLockError::Poisoned(_)) => {
                    panic!("Cannot continue: A previously run thread has panicked.")
                },
            }
        }).collect();
        self.panic_monitor.push(mutex);

        let order_in_progress = {
            let mut locked = self.orders_in_progress.lock().unwrap();
            if locked.contains(&order) {
                println!("order {:?} already in progress: nothing to do.", &order);
                true
            } else {
                locked.insert(order.clone());
                false
            }
        };

        if !order_in_progress {
            let channels_cloned = Arc::clone(&self.channels);
            let mut providers_cloned: Vec<J::P> = self.providers.lock().unwrap().clone();
            let mut provider_failures_cloned = Arc::clone(&self.provider_failures);
            let mut providers_in_use_cloned = Arc::clone(&self.providers_in_use);
            let orders_in_progress = Arc::clone(&self.orders_in_progress);
            let order_cloned = order;
            let (tx, rx) = channel::<FlexoMessage<J::P>>();
            let t = thread::spawn(move || {
                let _lock = mutex_cloned.lock().unwrap();
                let order = order_cloned.clone();
                let result = order.try_until_success(
                    &mut providers_cloned,
                    &mut provider_failures_cloned,
                    &mut providers_in_use_cloned,
                    channels_cloned.clone(),
                    tx
                );
                orders_in_progress.lock().unwrap().remove(&order_cloned);
                match result {
                    JobResult::Complete(mut complete_job) => {
                        let mut channels_cloned = channels_cloned.lock().unwrap();
                        complete_job.channel.reset();
                        let state = complete_job.channel.channel_state();
                        channels_cloned.insert(complete_job.provider.clone(), complete_job.channel);
                        JobOutcome::Success(complete_job.provider.clone(), state)
                    }
                    JobResult::Partial(JobPartiallyCompleted { channel, .. }) => {
                        let provider_failures = provider_failures_cloned.lock().unwrap().clone();
                        JobOutcome::Error(provider_failures, channel.channel_state())
                    }
                    JobResult::Error(JobTerminated { mut channel, .. } ) => {
                        channel.reset();
                        let provider_failures = provider_failures_cloned.lock().unwrap().clone();
                        JobOutcome::Error(provider_failures, channel.channel_state())
                    }
                }
            });

            ScheduleOutcome::Scheduled(ScheduledItem { join_handle: t, rx })
        } else {
            ScheduleOutcome::Skipped
        }
    }
}
