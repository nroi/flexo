#[macro_use] extern crate log;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::thread;
use std::thread::JoinHandle;
use std::collections::hash_map::Entry;
use crossbeam::crossbeam_channel::{unbounded, Sender, Receiver};

const NUM_MAX_ATTEMPTS: i32 = 100;

#[derive(Debug)]
pub struct JobPartiallyCompleted<J> where J: Job {
    pub channel: J::C,
    pub continue_at: u64
}

#[derive(Debug)]
pub struct JobTerminated<J> where J: Job {
    pub channel: J::C,
    pub error: J::E,
}

impl <J> JobPartiallyCompleted<J> where J: Job {
    pub fn new(channel: J::C, continue_at: u64) -> Self {
        Self {
            channel,
            continue_at
        }
    }
}

#[derive(Debug)]
pub struct JobCompleted<J> where J: Job {
    pub channel: J::C,
    pub provider: J::P,
    pub size: i64,
}

impl <J> JobCompleted<J> where J: Job {
    pub fn new(channel: J::C, provider: J::P, size: i64) -> Self {
        Self {
            channel,
            provider,
            size,
        }
    }
}

#[derive(Debug)]
pub enum JobResult<J> where J: Job {
    Complete(JobCompleted<J>),
    Partial(JobPartiallyCompleted<J>),
    Error(JobTerminated<J>),
    /// No provider was able to fulfil the order since the order was unavailable at all providers.
    Unavailable(J::C),
    /// The client has specified an invalid order that cannot be served.
    ClientError,
    /// An unexpected internal error has occurred while attempting to process the client's order.
    UnexpectedInternalError,
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub enum JobOutcome <J> where J: Job {
    Success(J::P),
    Error(HashMap<J::P, i32>),
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
    fn new_job(&self, properties: &<<Self as Provider>::J as Job>::PR, order: <<Self as Provider>::J as Job>::O) -> Self::J;

    /// returns an identifier that remains unchanged throughout the lifetime of the program.
    /// the intention is that while some properties of the provider change (i.e., its score),
    /// we still need to be able to recognize: Although those two Providers are not equal (p1 != p2),
    /// they actually refer to the same provider (p1.identity() = p2.identity()).
    fn identifier(&self) -> &<<Self as Provider>::J as Job>::PI;
    fn score(&self) -> <<Self as Provider>::J as Job>::S;

    fn punish(self, mut failures: MutexGuard<HashMap<Self, i32>>) {
        let value = failures.entry(self).or_insert(0);
        *value += 1;
    }

    fn reward(self, mut failures: MutexGuard<HashMap<Self, i32>>) {
        let value = failures.entry(self).or_insert(0);
        *value -= 1;
    }
}

pub trait Job where Self: std::marker::Sized + std::fmt::Debug + std::marker::Send + 'static {
    type S: std::cmp::Ord;
    type JS: JobState<J=Self>;
    type C: Channel<J=Self>;
    type O: Order<J=Self> + std::clone::Clone + std::cmp::Eq + std::hash::Hash;
    type P: Provider<J=Self>;
    type E: std::fmt::Debug;
    type PI: std::cmp::Eq;
    type PR: Properties + std::marker::Send + std::marker::Sync + std::clone::Clone;
    type OE: std::fmt::Debug;

    fn provider(&self) -> &Self::P;
    fn order(&self) -> Self::O;
    fn properties(&self)-> Self::PR;
    fn initialize_cache(properties: Self::PR) -> HashMap<Self::O, OrderState>;
    fn serve_from_provider(self, channel: Self::C, properties: Self::PR, cached_size: u64) -> JobResult<Self>;
    fn handle_error(self, error: Self::OE) -> JobResult<Self>;

    fn get_channel(&self, channels: &Arc<Mutex<HashMap<Self::P, Self::C>>>, tx: Sender<FlexoProgress>, last_chance: bool) -> Result<(Self::C, ChannelEstablishment), Self::OE> {
        let mut channels = channels.lock().unwrap();
        match channels.remove(&self.provider()) {
            Some(mut channel) => {
                println!("Reusing previous channel: {:?}", &self.provider());
                let result = channel.reset_order(self.order(), tx);
                result.map(|_| {
                    (channel, ChannelEstablishment::ExistingChannel)
                })
            }
            None => {
                println!("need to create new channel: {:?}", &self.provider());
                let channel = self.order().new_channel(self.properties(), tx, last_chance);
                channel.map(|c| {
                    (c, ChannelEstablishment::NewChannel)
                })
            }
        }
    }
}


pub trait Order where Self: std::marker::Sized + std::clone::Clone + std::cmp::Eq + std::hash::Hash + std::fmt::Debug + std::marker::Send + 'static {
    type J: Job<O=Self>;
    fn new_channel(self,
                   properties: <<Self as Order>::J as Job>::PR,
                   tx: Sender<FlexoProgress>,
                   last_chance: bool
    ) -> Result<<<Self as Order>::J as Job>::C, <<Self as Order>::J as Job>::OE>;

    /// Returns true if this order can be served from cache, false otherwise.
    fn is_cacheable(&self) -> bool;

    fn is_cached(&self, cached: MutexGuard<HashMap<Self, OrderState>>) -> bool {
        match &cached.get(self) {
            Some(OrderState::Cached(_size)) => {
                // TODO We also need to ensure that the size of the cached file is equal to the content length
                // of the file to retrieve.
                true
            },
            _ => false,
        }
    }

    fn try_until_success(
        self,
        mut providers: &mut Vec<<<Self as Order>::J as Job>::P>,
        provider_failures: &mut Arc<Mutex<HashMap<<<Self as Order>::J as Job>::P , i32>>>,
        provider_current_usages: &mut Arc<Mutex<HashMap<<<Self as Order>::J as Job>::P, i32>>>,
        channels: Arc<Mutex<HashMap<<<Self as Order>::J as Job>::P, <<Self as Order>::J as Job>::C>>>,
        tx: Sender<FlexoMessage<<<Self as Order>::J as Job>::P>>,
        tx_progress: Sender<FlexoProgress>,
        properties: <<Self as Order>::J as Job>::PR,
        cached_size: u64,
    ) -> JobResult<Self::J> {
        let mut num_attempt = 0;
        let mut punished_providers = Vec::new();
        let result = loop {
            num_attempt += 1;
            let (provider, is_last_provider) =
                self.select_provider(&mut providers, provider_current_usages.lock().unwrap(), provider_failures.lock().unwrap());
            let last_chance = num_attempt >= NUM_MAX_ATTEMPTS || is_last_provider;
            let message = FlexoMessage::ProviderSelected(provider.clone());
            let _ = tx.send(message);
            {
                let mut provider_current_usages = provider_current_usages.lock().unwrap();
                let value = provider_current_usages.entry(provider.clone()).or_insert(0);
                *value += 1;
            }
            debug!("selected provider: {:?}", &provider);
            let self_cloned: Self = self.clone();
            let job = provider.new_job(&properties, self_cloned);
            debug!("Attempt to obtain new channel");
            let channel_result = job.get_channel(&channels, tx_progress.clone(), last_chance);
            let result = match channel_result {
                Ok((c, ce)) => {
                    let _ = tx.send(FlexoMessage::ChannelEstablished(ce));
                    job.serve_from_provider(c, properties.clone(), cached_size)
                }
                Err(e) => {
                    dbg!(&e);
                    let _ = tx.send(FlexoMessage::OrderError);
                    let _ = tx_progress.send(FlexoProgress::OrderError);
                    job.handle_error(e)
                }
            };
            match &result {
                JobResult::Complete(_) => {
                    provider.clone().reward(provider_failures.lock().unwrap());
                },
                JobResult::Partial(partial_job) => {
                    provider.clone().punish(provider_failures.lock().unwrap());
                    punished_providers.push(provider.clone());
                    println!("Job only partially finished until size {:?}", partial_job.continue_at);
                },
                JobResult::Error(e) => {
                    provider.clone().punish(provider_failures.lock().unwrap());
                    punished_providers.push(provider.clone());
                    println!("Error: {:?}, try again", e)
                },
                JobResult::Unavailable(_) => {
                    println!("Order is not available, let's try again with a different provider.")
                },
                JobResult::ClientError => {
                    break result;
                },
                JobResult::UnexpectedInternalError => {
                    break result;
                },
            };
            if result.is_success() || providers.is_empty() || last_chance {
                break result;
            }
        };
        if !result.is_success() {
            Self::pardon(punished_providers, provider_failures.lock().unwrap());
        }

        result
    }

    fn select_provider(
        &self,
        providers: &mut Vec<<<Self as Order>::J as Job>::P>,
        provider_current_usages: MutexGuard<HashMap<<<Self as Order>::J as Job>::P, i32>>,
        provider_failures: MutexGuard<HashMap<<<Self as Order>::J as Job>::P, i32>>,
    ) -> (<<Self as Order>::J as Job>::P, bool) {
        let (idx, (_, _, _, _)) = providers
            .iter()
            .map(|x| (provider_failures.get(&x).unwrap_or(&0), provider_current_usages.get(&x).unwrap_or(&0), x.score(), x))
            .enumerate()
            .min_by(|(_idx_x, (num_failures_x, num_usages_x, score_x, _x)),
                     (_idx_y, (num_failures_y, num_usages_y, score_y, _y))|
                (num_failures_x, num_usages_x, score_x).cmp(&(num_failures_y, num_usages_y, score_y)))
            .unwrap();

        let provider = providers.remove(idx);
        (provider, providers.is_empty())
    }

    fn pardon(punished_providers: Vec<<<Self as Order>::J as Job>::P>,
              mut failures: MutexGuard<HashMap<<<Self as Order>::J as Job>::P, i32>>)
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
}

pub trait Channel where Self: std::marker::Sized + std::fmt::Debug + std::marker::Send + 'static {
    type J: Job;

    fn progress_indicator(&self) -> Option<u64>;
    fn reset_order(&mut self, order: <<Self as Channel>::J as Job>::O, tx: Sender<FlexoProgress>) -> Result<(), <<Self as Channel>::J as Job>::OE>;
    fn job_state_item(&mut self) -> &mut JobStateItem<Self::J>;

    /// After a job has completed, all stateful information associated with this particular job should be dropped.
    fn reset_job_state(&mut self) {
        self.job_state_item().reset();
    }
}

#[derive(PartialEq, Eq, Hash, Clone, Debug, Copy)]
pub enum ChannelEstablishment {
    NewChannel,
    ExistingChannel,
}

/// Marker trait.
pub trait JobState {
    type J: Job;
}

/// Marker trait.
pub trait Properties {}

#[derive(Debug)]
pub struct JobStateItem<J> where J: Job {
    pub order: J::O,
    /// Used to manage the resources acquired for a job. It is set to Some(_) if there is an active job associated
    /// with the Channel, or None if the channel is just kept open for requests that may arrive in the future. The
    /// reason for using Optional (rather than just JS) is that this way, drop() will called on the JS as soon as we
    /// reset the state to None, so that acquired resources are released as soon as possible.
    pub job_state: Option<J::JS>,
    pub tx: Sender<FlexoProgress>,
}

impl <J> JobStateItem<J> where J: Job {
    fn reset(&mut self) {
        self.job_state = None
    }
}

#[derive(PartialEq, Eq, Hash, Clone, Debug, Copy)]
pub struct CachedItem {
    pub complete_size: u64,
    pub cached_size: u64,
}

#[derive(PartialEq, Eq, Hash, Clone, Debug, Copy)]
pub enum OrderState {
    Cached(CachedItem),
    InProgress
}

/// The context in which a job is executed, including all stateful information required by the job.
/// This context is meant to be initialized once during the program's lifecycle.
pub struct JobContext<J> where J: Job, {
    providers: Arc<Mutex<Vec<J::P>>>,
    channels: Arc<Mutex<HashMap<J::P, J::C>>>,
    order_states: Arc<Mutex<HashMap<J::O, OrderState>>>,
    providers_in_use: Arc<Mutex<HashMap<J::P, i32>>>,
    panic_monitor: Vec<Arc<Mutex<i32>>>,
    provider_failures: Arc<Mutex<HashMap<J::P, i32>>>,
    pub properties: J::PR
}

pub struct ScheduledItem<J> where J: Job {
    pub join_handle: JoinHandle<JobOutcome<J>>,
    pub rx: Receiver<FlexoMessage<J::P>>,
    pub rx_progress: Receiver<FlexoProgress>,
}

pub enum ScheduleOutcome<J> where J: Job {
    /// The order is already in progress, no new order was scheduled.
    AlreadyInProgress,
    /// The order has to be fetched from a provider.
    Scheduled(ScheduledItem<J>),
    /// The order is already available in the cache.
    Cached,
    /// the order cannot be served from cache
    Uncacheable(J::P),
}

#[derive(PartialEq, Eq, Hash, Clone, Debug)]
pub enum FlexoMessage <P> {
    ProviderSelected(P),
    ChannelEstablished(ChannelEstablishment),
    OrderError,
}

#[derive(PartialEq, Eq, Hash, Clone, Debug)]
pub enum FlexoProgress {
    /// The job cannot be completed because the requested order is not available.
    Unavailable,
    JobSize(u64),
    Progress(u64),
    Completed,
    OrderError,
}


impl <J> JobContext<J> where J: Job {
    pub fn new(initial_providers: Vec<J::P>, properties: J::PR) -> Self {
        let providers: Arc<Mutex<Vec<J::P>>> = Arc::new(Mutex::new(initial_providers));
        let channels: Arc<Mutex<HashMap<J::P, J::C>>> = Arc::new(Mutex::new(HashMap::new()));
        let order_states: Arc<Mutex<HashMap<J::O, OrderState>>> = Arc::new(Mutex::new(J::initialize_cache(properties.clone())));
        let providers_in_use: Arc<Mutex<HashMap<J::P, i32>>> = Arc::new(Mutex::new(HashMap::new()));
        let provider_records: Arc<Mutex<HashMap<J::P, i32>>> = Arc::new(Mutex::new(HashMap::new()));
        let thread_mutexes: Vec<Arc<Mutex<i32>>> = Vec::new();
        Self {
            providers,
            channels,
            order_states,
            provider_failures: provider_records,
            providers_in_use,
            panic_monitor: thread_mutexes,
            properties,
        }
    }

    fn best_provider(&self) -> J::P {
        // Providers are assumed to be sorted in ascending order from best to worst.
        let providers: Vec<J::P> = self.providers.lock().unwrap().to_vec();
        providers[0].clone()
    }

    /// Schedule the order, or return info on why scheduling this order is not possible or not necessary.
    pub fn try_schedule(&mut self, order: J::O, resume_from: Option<u64>) -> ScheduleOutcome<J> {
        if !order.is_cacheable() {
            return ScheduleOutcome::Uncacheable(self.best_provider());
        }
        let resume_from = resume_from.unwrap_or(0);
        let cached_size: u64 = {
            let mut order_states = self.order_states.lock().unwrap();
            let cached_size = match order_states.get(&order) {
                None if resume_from > 0 => {
                    // Cannot serve this order from cache: See issue #7
                    return ScheduleOutcome::Uncacheable(self.best_provider());
                }
                None => 0,
                Some(OrderState::Cached(CachedItem { cached_size, .. } )) if cached_size < &resume_from => {
                    // Cannot serve this order from cache: See issue #7
                    return ScheduleOutcome::Uncacheable(self.best_provider());
                },
                Some(OrderState::Cached(CachedItem { complete_size, cached_size } )) if complete_size == cached_size => {
                    return ScheduleOutcome::Cached;
                },
                Some(OrderState::Cached(CachedItem { cached_size, .. } )) => *cached_size,
                Some(OrderState::InProgress) => {
                    println!("order {:?} already in progress: nothing to do.", &order);
                    return ScheduleOutcome::AlreadyInProgress;
                }
            };
            order_states.insert(order.clone(), OrderState::InProgress);
            cached_size
        };
        self.schedule(order, cached_size)
    }

    /// Schedules the job so that the order will be fetched from the provider.
    fn schedule(&mut self, order: J::O, cached_size: u64) -> ScheduleOutcome<J> {
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

        let (tx, rx) = unbounded::<FlexoMessage<J::P>>();
        let (tx_progress, rx_progress) = unbounded::<FlexoProgress>();
        let channels_cloned = Arc::clone(&self.channels);
        let mut providers_cloned: Vec<J::P> = self.providers.lock().unwrap().clone();
        let mut provider_failures_cloned = Arc::clone(&self.provider_failures);
        let mut providers_in_use_cloned = Arc::clone(&self.providers_in_use);
        let order_states = Arc::clone(&self.order_states);
        let order_cloned = order.clone();
        let properties = self.properties.clone();
        let t = thread::spawn(move || {
            let _lock = mutex_cloned.lock().unwrap();
            let order: <J as Job>::O = order.clone();
            let result = order.try_until_success(
                &mut providers_cloned,
                &mut provider_failures_cloned,
                &mut providers_in_use_cloned,
                channels_cloned.clone(),
                tx,
                tx_progress,
                properties,
                cached_size,
            );
            order_states.lock().unwrap().remove(&order_cloned);
            match result {
                JobResult::Complete(mut complete_job) => {
                    complete_job.channel.reset_job_state();
                    let mut channels_cloned = channels_cloned.lock().unwrap();
                    channels_cloned.insert(complete_job.provider.clone(), complete_job.channel);
                    let cached_item = CachedItem {
                        complete_size: complete_job.size as u64,
                        cached_size: complete_job.size as u64,
                    };
                    let order_state = OrderState::Cached(cached_item);
                    order_states.lock().unwrap().insert(order_cloned.clone(), order_state);
                    JobOutcome::Success(complete_job.provider.clone())
                }
                JobResult::Partial(JobPartiallyCompleted { mut channel, .. }) => {
                    channel.reset_job_state();
                    let provider_failures = provider_failures_cloned.lock().unwrap().clone();
                    JobOutcome::Error(provider_failures)
                }
                JobResult::Error(JobTerminated { mut channel, .. } ) => {
                    channel.reset_job_state();
                    let provider_failures = provider_failures_cloned.lock().unwrap().clone();
                    JobOutcome::Error(provider_failures)
                }
                JobResult::Unavailable(mut channel) => {
                    println!("The given order was unavailable for all providers.");
                    channel.reset_job_state();
                    let provider_failures = provider_failures_cloned.lock().unwrap().clone();
                    JobOutcome::Error(provider_failures)
                }
                JobResult::ClientError => {
                    // TODO unclear what to do here. We're calling "reset_job_state" in all other branches,
                    // is something similar required here, too?
                    let provider_failures = provider_failures_cloned.lock().unwrap().clone();
                    JobOutcome::Error(provider_failures)
                }
                JobResult::UnexpectedInternalError => {
                    // TODO unclear what to do here. We're calling "reset_job_state" in all other branches,
                    // is something similar required here, too?
                    let provider_failures = provider_failures_cloned.lock().unwrap().clone();
                    JobOutcome::Error(provider_failures)
                }
            }
        });

        ScheduleOutcome::Scheduled(ScheduledItem { join_handle: t, rx, rx_progress })
    }
}
