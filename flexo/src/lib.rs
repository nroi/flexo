#[macro_use] extern crate log;

use std::collections::{HashMap, HashSet};
use std::collections::hash_map::Entry;
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::thread;
use std::thread::JoinHandle;
use std::sync::atomic::{AtomicU32, Ordering};

use crossbeam::channel::{Receiver, Sender, unbounded};

static LOGICAL_CLOCK: AtomicU32 = AtomicU32::new(1);

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
    Error(HashMap<J::P, ProviderMetrics>),
}

impl <J> JobResult<J> where J: Job {
    fn is_success(&self) -> bool {
        matches!(self, JobResult::Complete(_))
    }
}

pub trait Provider where
    Self: std::marker::Sized + std::fmt::Debug + std::clone::Clone + std::cmp::Eq + std::hash::Hash + std::marker::Send + 'static,
{
    type J: Job;
    fn new_job(
        &self,
        properties: &<<Self as Provider>::J as Job>::PR,
        order: <<Self as Provider>::J as Job>::O
    ) -> Self::J;

    fn initial_score(&self) -> <<Self as Provider>::J as Job>::S;

    /// A short description which will be used in log messages.
    fn description(&self) -> String;

    fn punish(self, mut provider_metrics: MutexGuard<HashMap<Self, ProviderMetrics>>) {
        provider_metrics.entry(self)
            .and_modify(|p| p.num_failures += 1)
            .or_insert(ProviderMetrics::default());
    }

}

pub trait Job where Self: std::marker::Sized + std::fmt::Debug + std::marker::Send + 'static {
    type S: std::cmp::Ord + core::marker::Copy + std::fmt::Debug;
    type JS;
    type C: Channel<J=Self>;
    type O: Order<J=Self> + std::clone::Clone + std::cmp::Eq + std::hash::Hash + std::fmt::Debug;
    type P: Provider<J=Self>;
    type E: std::fmt::Debug;
    type PI: std::cmp::Eq;
    type PR: Properties + std::marker::Send + std::marker::Sync + std::clone::Clone;
    type OE: std::fmt::Debug;

    fn provider(&self) -> &Self::P;
    fn order(&self) -> Self::O;
    fn properties(&self)-> Self::PR;
    fn cache_state(order: &<Self as Job>::O, properties: &Self::PR) -> Option<CachedItem>;
    fn serve_from_provider(self, channel: Self::C, properties: &Self::PR, cached_size: u64) -> JobResult<Self>;
    fn handle_error(self, error: Self::OE) -> JobResult<Self>;
    fn acquire_resources(order: &Self::O, properties: &Self::PR, last_chance: bool) -> std::io::Result<Self::JS>;

    fn get_channel(
        &self,
        channels: &Arc<Mutex<HashMap<Self::P, Self::C>>>,
        tx: Sender<FlexoProgress>,
        last_chance: bool
    ) -> Result<(Self::C, ChannelEstablishment), Self::OE> {
        let mut channels = channels.lock().unwrap();
        match channels.remove(&self.provider()) {
            Some(channel) => {
                debug!("Attempt to reuse previous connection from {}", &self.provider().description());
                let result = self.order().reuse_channel(self.properties(), tx, last_chance, channel);
                result.map(|new_channel| {
                    (new_channel, ChannelEstablishment::ExistingChannel)
                })
            }
            None => {
                debug!("Establish a new connection to {:?}", &self.provider().description());
                let channel = self.order().new_channel(self.properties(), tx, last_chance);
                channel.map(|c| {
                    (c, ChannelEstablishment::NewChannel)
                })
            }
        }
    }
}

pub struct ProvidersWithMetrics<J> where J: Job {
    pub providers: Vec<J::P>,
    pub provider_metrics: Arc<Mutex<HashMap<J::P, ProviderMetrics>>>
}

pub trait Order where Self: std::marker::Sized + std::clone::Clone + std::cmp::Eq + std::hash::Hash + std::fmt::Debug + std::marker::Send + 'static {
    type J: Job<O=Self>;
    fn new_channel(
        self,
        properties: <<Self as Order>::J as Job>::PR,
        tx: Sender<FlexoProgress>,
        last_chance: bool,
    ) -> Result<<<Self as Order>::J as Job>::C, <<Self as Order>::J as Job>::OE>;

    fn reuse_channel(
        self,
        properties: <<Self as Order>::J as Job>::PR,
        tx: Sender<FlexoProgress>,
        last_chance: bool,
        channel: <<Self as Order>::J as Job>::C,
    ) -> Result<<<Self as Order>::J as Job>::C, <<Self as Order>::J as Job>::OE>;

    fn is_cacheable(&self) -> bool;

    fn retryable(&self) -> bool {
        true
    }

    fn description(&self) -> &str;

    fn try_until_success(
        self,
        provider_stats: &mut ProvidersWithMetrics<<Self as Order>::J>,
        custom_provider: Option<<<Self as Order>::J as Job>::P>,
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
            debug!("Attempt number {}", num_attempt);
            let (provider, is_last_provider) = self.select_provider(provider_stats, &custom_provider);
            debug!("Trying to serve {} via {:?}", &self.description(), &provider);
            debug!("No providers are left after this provider? {}", is_last_provider);
            let last_chance = num_attempt >= NUM_MAX_ATTEMPTS || is_last_provider || !self.retryable();
            let message = FlexoMessage::ProviderSelected(provider.clone());
            let _ = tx.send(message);
            let self_cloned: Self = self.clone();
            let job = provider.new_job(&properties, self_cloned);
            debug!("Attempt to establish new connection");
            let channel_result = job.get_channel(&channels, tx_progress.clone(), last_chance);
            let result = match channel_result {
                Ok((channel, channel_establishment)) => {
                    let _ = tx.send(FlexoMessage::ChannelEstablished(channel_establishment));
                    job.serve_from_provider(channel, &properties, cached_size)
                }
                Err(e) => {
                    warn!("Error while attempting to establish a new connection: {:?}", e);
                    let _ = tx.send(FlexoMessage::OrderError);
                    let _ = tx_progress.send(FlexoProgress::OrderError);
                    job.handle_error(e)
                }
            };
            match &result {
                JobResult::Complete(_) => {
                    debug!("Job completed with provider {}", provider.description());
                },
                JobResult::Partial(partial_job) => {
                    provider.clone().punish(provider_stats.provider_metrics.lock().unwrap());
                    punished_providers.push(provider.clone());
                    debug!("Job only partially finished until size {:?}", partial_job.continue_at);
                },
                JobResult::Error(e) => {
                    provider.clone().punish(provider_stats.provider_metrics.lock().unwrap());
                    punished_providers.push(provider.clone());
                    info!("Error: {:?}, try again", e)
                },
                JobResult::Unavailable(_) => {
                    info!("{} is not available at {}", &self.description(), provider.description());
                },
                JobResult::ClientError => {
                    warn!("Unable to finish job: {:?}", &result);
                    break result;
                },
                JobResult::UnexpectedInternalError => {
                    warn!("Unable to finish job: {:?}", &result);
                    break result;
                },
            };
            if result.is_success() || provider_stats.providers.is_empty() || last_chance {
                break result;
            }
        };
        if !result.is_success() {
            Self::pardon(punished_providers, provider_stats.provider_metrics.lock().unwrap());
        }

        result
    }

    fn select_provider<'a>(
        &self,
        provider_stats: &'a ProvidersWithMetrics<<Self as Order>::J>,
        custom_provider: &'a Option<<<Self as Order>::J as Job>::P>,
    ) -> (&'a <<Self as Order>::J as Job>::P, bool) {
        match custom_provider {
            Some(p) => (p, true),
            None => {
                let mut provider_metrics = provider_stats.provider_metrics.lock().unwrap();
                let (idx, _) = provider_stats.providers
                    .iter()
                    .map(|provider| {
                        let metric = *(provider_metrics.get(provider)).unwrap_or(&ProviderMetrics::default());
                        DynamicScore {
                            num_failures: metric.num_failures,
                            most_recent_usage: metric.most_recent_usage,
                            initial_score: provider.initial_score(),
                        }
                    })
                    .enumerate()
                    .min_by_key(|(_idx, dynamic_score)| *dynamic_score)
                    .unwrap();
                let provider = provider_stats.providers.get(idx).unwrap();
                debug!("Selected provider: {:?}", provider);
                // TODO make sure this compiles on a raspberry pi
                let timestamp = LOGICAL_CLOCK.fetch_add(1, Ordering::Relaxed);
                provider_metrics.entry(provider.clone())
                    .and_modify(|e| e.most_recent_usage = timestamp);

                (provider, provider_stats.providers.is_empty())
            }
        }
    }

    fn pardon(
        punished_providers: Vec<<<Self as Order>::J as Job>::P>,
        mut provider_metrics: MutexGuard<HashMap<<<Self as Order>::J as Job>::P, ProviderMetrics>>,
    ) {
        for not_guilty in punished_providers {
            match (*provider_metrics).entry(not_guilty.clone()) {
                Entry::Occupied(mut value) => {
                    let value = value.get_mut();
                    value.num_failures -= 1;
                },
                Entry::Vacant(_) => {},
            }
        }
    }
}

/// A score that incorporates information that we have gained while using this provider.
#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Debug)]
struct DynamicScore <S> where S: Ord {
    num_failures: u32,
    most_recent_usage: u32,
    initial_score: S,
}

pub trait Channel where Self: std::marker::Sized + std::fmt::Debug + std::marker::Send + 'static {
    type J: Job;

    fn progress_indicator(&self) -> Option<u64>;
    fn job_state(&mut self) -> &mut JobState<Self::J>;
}

#[derive(PartialEq, Eq, Hash, Clone, Debug, Copy)]
pub enum ChannelEstablishment {
    NewChannel,
    ExistingChannel,
}

/// Marker trait.
pub trait Properties {}

#[derive(Debug)]
pub struct JobState<J> where J: Job {
    pub order: J::O,
    /// Used to manage the resources acquired for a job. It is set to Some(_) if there is an active job associated
    /// with the Channel, or None if the channel is just kept open for requests that may arrive in the future. The
    /// reason for using Optional (rather than just JS) is that this way, drop() will called on the JS as soon as we
    /// reset the state to None, so that acquired resources are released as soon as possible.
    pub job_resources: Option<J::JS>,
    pub tx: Sender<FlexoProgress>,
}

impl <J> JobState<J> where J: Job {
    /// Release all resources (e.g. opened files) that were required for this particular job.
    fn release_job_resources(&mut self) {
        self.job_resources = None
    }
}

#[derive(PartialEq, Eq, Hash, Clone, Debug, Copy)]
pub struct CachedItem {
    pub complete_size: Option<u64>,
    pub cached_size: u64,
}

#[derive(PartialEq, Eq, Hash, Clone, Debug, Copy)]
pub enum OrderState {
    Cached(CachedItem),
    InProgress
}

/// The context in which a job is executed, including all stateful information required by the job.
/// This context is meant to be initialized once during the program's lifecycle.
pub struct JobContext<J> where J: Job {
    providers: Arc<Mutex<Vec<J::P>>>,
    channels: Arc<Mutex<HashMap<J::P, J::C>>>,
    orders_in_progress: Arc<Mutex<HashSet<J::O>>>,
    provider_metrics: Arc<Mutex<HashMap<J::P, ProviderMetrics>>>,
    panic_monitor: Vec<Arc<Mutex<i32>>>,
    pub properties: J::PR,
}

#[derive(PartialEq, Eq, Hash, Clone, Debug, Copy, Default)]
pub struct ProviderMetrics {
    most_recent_usage: u32,
    num_failures: u32,
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
    /// the order cannot be cached
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
        let orders_in_progress: Arc<Mutex<HashSet<J::O>>> = Arc::new(Mutex::new(HashSet::new()));
        let provider_metrics: Arc<Mutex<HashMap<J::P, ProviderMetrics>>> = Arc::new(Mutex::new(HashMap::new()));
        let thread_mutexes: Vec<Arc<Mutex<i32>>> = Vec::new();
        Self {
            providers,
            channels,
            orders_in_progress,
            provider_metrics,
            panic_monitor: thread_mutexes,
            properties,
        }
    }

    fn best_provider(&self, custom_provider: Option<J::P>) -> J::P {
        // TODO this looks awkward.
        match custom_provider {
            None => {
                // no custom provider is required to fulfil this order: We can just choose the best provider
                // among all available providers.
                // Providers are assumed to be sorted in ascending order from best to worst.
                let providers: Vec<J::P> = self.providers.lock().unwrap().to_vec();
                providers[0].clone()
            }
            Some(p) => {
                // This is a "special order" that needs to be served by a custom provider.
                // Speaking in Arch Linux terminology: This is a request that must be served
                // from a custom repository / unofficial repository.
                p
            }
        }
    }

    /// Schedule the order, or return info on why scheduling this order is not possible or not necessary.
    pub fn try_schedule(
        &mut self,
        order: J::O,
        custom_provider: Option<J::P>,
        resume_from: Option<u64>,
    ) -> ScheduleOutcome<J> {
        let resume_from = resume_from.unwrap_or(0);
        let cached_size: u64 = {
            let mut orders_in_progress = self.orders_in_progress.lock().unwrap();
            let cached_size = if orders_in_progress.contains(&order) {
                debug!("order {:?} already in progress: nothing to do.", &order);
                return ScheduleOutcome::AlreadyInProgress;
            } else {
                let cache_state_result = if order.is_cacheable() {
                    J::cache_state(&order, &self.properties)
                } else {
                    None
                };
                match cache_state_result {
                    None if resume_from > 0 => {
                        // Cannot store this order in cache: See issue #7
                        return ScheduleOutcome::Uncacheable(self.best_provider(custom_provider));
                    },
                    None => 0,
                    Some(CachedItem { cached_size, .. } ) if cached_size < resume_from => {
                        // Cannot serve this order from cache: See issue #7
                        return ScheduleOutcome::Uncacheable(self.best_provider(custom_provider));
                    },
                    Some(CachedItem { complete_size: Some(c), cached_size }) if c == cached_size => {
                        debug!("Order {:?} is already cached.", &order);
                        return ScheduleOutcome::Cached;
                    },
                    Some(CachedItem { cached_size, .. } ) => cached_size,
                }
            };
            orders_in_progress.insert(order.clone());
            cached_size
        };
        self.schedule(order, custom_provider, cached_size)
    }

    /// Schedules the job so that the order will be fetched from the provider.
    fn schedule(&mut self, order: J::O, custom_provider: Option<J::P>, cached_size: u64) -> ScheduleOutcome<J> {
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
        let providers_cloned: Vec<J::P> = self.providers.lock().unwrap().clone();
        let provider_metrics_cloned = Arc::clone(&self.provider_metrics);
        let order_states = Arc::clone(&self.orders_in_progress);
        let order_cloned = order.clone();
        let properties = self.properties.clone();

        let mut provider_stats = ProvidersWithMetrics {
            providers: providers_cloned,
            provider_metrics: provider_metrics_cloned,
        };
        let t = thread::spawn(move || {
            let _lock = mutex_cloned.lock().unwrap();
            let order: <J as Job>::O = order.clone();
            let result = order.try_until_success(
                &mut provider_stats,
                custom_provider,
                channels_cloned.clone(),
                tx,
                tx_progress,
                properties,
                cached_size,
            );
            order_states.lock().unwrap().remove(&order_cloned);
            match result {
                JobResult::Complete(mut complete_job) => {
                    complete_job.channel.job_state().release_job_resources();
                    let mut channels_cloned = channels_cloned.lock().unwrap();
                    channels_cloned.insert(complete_job.provider.clone(), complete_job.channel);
                    JobOutcome::Success(complete_job.provider)
                }
                JobResult::Partial(JobPartiallyCompleted { mut channel, .. }) => {
                    channel.job_state().release_job_resources();
                    let provider_metrics = provider_stats.provider_metrics.lock().unwrap().clone();
                    JobOutcome::Error(provider_metrics)
                }
                JobResult::Error(JobTerminated { mut channel, .. } ) => {
                    channel.job_state().release_job_resources();
                    let provider_metrics = provider_stats.provider_metrics.lock().unwrap().clone();
                    JobOutcome::Error(provider_metrics)
                }
                JobResult::Unavailable(mut channel) => {
                    info!("{} was unavailable at all remote mirrors.", &order_cloned.description());
                    channel.job_state().release_job_resources();
                    let provider_metrics = provider_stats.provider_metrics.lock().unwrap().clone();
                    JobOutcome::Error(provider_metrics)
                }
                JobResult::ClientError => {
                    let provider_metrics = provider_stats.provider_metrics.lock().unwrap().clone();
                    JobOutcome::Error(provider_metrics)
                }
                JobResult::UnexpectedInternalError => {
                    let provider_metrics = provider_stats.provider_metrics.lock().unwrap().clone();
                    JobOutcome::Error(provider_metrics)
                }
            }
        });

        ScheduleOutcome::Scheduled(ScheduledItem { join_handle: t, rx, rx_progress })
    }
}

#[test]
fn test_no_failures_preferred() {
    let s1 = DynamicScore {
        num_failures: 2,
        most_recent_usage: 23,
        initial_score: 0,
    };
    let s2 = DynamicScore {
        num_failures: 0,
        most_recent_usage: 0,
        initial_score: -1,
    };
    assert!(s2 < s1);
}

#[test]
fn test_initial_score_lower_is_better() {
    let s1 = DynamicScore {
        num_failures: 0,
        most_recent_usage: 0,
        initial_score: 0,
    };
    let s2 = DynamicScore {
        num_failures: 0,
        most_recent_usage: 0,
        initial_score: -1,
    };
    assert!(s2 < s1);
}

