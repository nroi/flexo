mod provider_guards;

#[macro_use] extern crate log;

use std::collections::{HashMap, HashSet};
use std::collections::hash_map::Entry;
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::{thread, fmt};
use std::thread::JoinHandle;
use serde::Serialize;
use std::time::{Instant, Duration};
use crossbeam::channel::{Receiver, Sender, unbounded};
use crate::provider_guards::{ProviderGuards, ProviderChoice, ProviderGuard};
use std::fmt::{Display, Formatter};

const NUM_MAX_ATTEMPTS: i32 = 25;

// It's important that this value is lower than the TIMEOUT_RECEIVE_CONTENT_LENGTH value:
// Otherwise, if we keep doing our retries for too long, the other thread stops waiting.
const TIMEOUT_ALL_RETRIES: Duration = Duration::from_secs(4);

pub const LOGICAL_CLOCK_INITIAL_VALUE: u32 = 1;

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
    Error(HashMap<ProviderIdentifier, ProviderMetrics>),
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

    /// The initial score (as opposed to the dynamic score) is obtained using information that is available without ever
    /// having used this provider ourselves. It does not include our own experience with this provider, for example, a
    /// provider can have an excellent initial_score, but for some reason, fail miserably when we try to use it.
    /// The initial_score should give some insight as to which providers are worth trying out, while the dynamic score
    /// is used to subsequently filter out bad providers among those providers with a good initial_score.
    fn initial_score(&self) -> <<Self as Provider>::J as Job>::S;

    /// A unique identifier
    fn identifier(&self) -> ProviderIdentifier;

    fn punish(&self, mut provider_metrics: MutexGuard<HashMap<ProviderIdentifier, ProviderMetrics>>) {
        provider_metrics.entry(self.identifier())
            .and_modify(|p| p.num_failures += 1)
            .or_insert_with(ProviderMetrics::default);
    }
}

#[derive(PartialEq, Eq, Hash, Clone, Debug)]
pub struct ProviderIdentifier {
    pub identifier: String,
}

impl Display for ProviderIdentifier {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        self.identifier.fmt(f)
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
    type DSU: DynamicScoreUncacheable<Self::S> + std::marker::Copy;
    type DSC: DynamicScoreCacheable<Self::S> + std::marker::Copy;

    fn provider(&self) -> &Self::P;
    fn order(&self) -> Self::O;
    fn properties(&self)-> Self::PR;
    fn cache_state(order: &<Self as Job>::O, properties: &Self::PR) -> Option<CachedItem>;
    fn serve_from_provider(self, channel: Self::C, properties: &Self::PR) -> JobResult<Self>;
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
                debug!("Attempt to reuse previous connection from {}", &self.provider().identifier());
                let result = self.order().reuse_channel(self.properties(), tx, last_chance, channel);
                result.map(|new_channel| {
                    (new_channel, ChannelEstablishment::ExistingChannel)
                })
            }
            None => {
                debug!("Establish a new connection to {}", &self.provider().identifier());
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
        provider_guards: Arc<ProviderGuards<<<Self as Order>::J as Job>::P>>,
        provider_metrics: &mut Arc<Mutex<HashMap<ProviderIdentifier, ProviderMetrics>>>,
        custom_provider: Option<<<Self as Order>::J as Job>::P>,
        channels: Arc<Mutex<HashMap<<<Self as Order>::J as Job>::P, <<Self as Order>::J as Job>::C>>>,
        tx_integration_test: Sender<IntegrationTestMessage>,
        tx_progress: Sender<FlexoProgress>,
        properties: <<Self as Order>::J as Job>::PR,
    ) -> JobResult<Self::J> {
        let mut num_attempt = 0;
        let mut punished_providers = Vec::new();
        let start_time = Instant::now();
        let mut unsuccessful_providers = HashSet::<ProviderIdentifier>::new();
        let result = loop {
            num_attempt += 1;
            debug!("Attempt number {}", num_attempt);
            if num_attempt > 1 && start_time.elapsed() > TIMEOUT_ALL_RETRIES {
                warn!("Unable to complete attempt number {}: The timeout has elapsed.", num_attempt);
            }
            let (provider_guard, is_last_provider) = self.select_provider(
                &provider_guards,
                &mut provider_metrics.lock().unwrap(),
                &custom_provider,
                &unsuccessful_providers,
            );
            debug!("Trying to serve {} via {}", &self.description(), provider_guard.guarded_provider.identifier());
            debug!("No providers are left after this provider? {}", is_last_provider);
            let last_chance = num_attempt >= NUM_MAX_ATTEMPTS || is_last_provider || !self.retryable();
            send(
                IntegrationTestMessage::ProviderSelected(provider_guard.guarded_provider.identifier()),
                &tx_integration_test
            );
            let self_cloned: Self = self.clone();
            let job = provider_guard.guarded_provider.new_job(&properties, self_cloned);
            debug!("Attempt to establish new connection");
            let channel_result = job.get_channel(&channels, tx_progress.clone(), last_chance);
            let result = match channel_result {
                Ok((channel, channel_establishment)) => {
                    send(
                        IntegrationTestMessage::ChannelEstablished(channel_establishment),
                        &tx_integration_test
                    );
                    job.serve_from_provider(channel, &properties)
                }
                Err(e) => {
                    warn!("Error while attempting to establish a new connection: {:?}", e);
                    send(
                        IntegrationTestMessage::OrderError,
                        &tx_integration_test
                    );
                    let _ = tx_progress.send(FlexoProgress::OrderError);
                    job.handle_error(e)
                }
            };
            match &result {
                JobResult::Complete(_) => {
                    debug!("Job completed with provider {}", provider_guard.guarded_provider.identifier());
                },
                JobResult::Partial(partial_job) => {
                    provider_guard.guarded_provider.punish(provider_metrics.lock().unwrap());
                    punished_providers.push(provider_guard.guarded_provider.identifier());
                    debug!("Job only partially finished until size {:?}", partial_job.continue_at);
                },
                JobResult::Error(e) => {
                    provider_guard.guarded_provider.punish(provider_metrics.lock().unwrap());
                    punished_providers.push(provider_guard.guarded_provider.identifier());
                    info!("Error: {:?}, try again", e)
                },
                JobResult::Unavailable(_) => {
                    info!("{} is not available at {}",
                          &self.description(), provider_guard.guarded_provider.identifier());
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
            if result.is_success() || last_chance {
                break result;
            }
            if !result.is_success() {
                unsuccessful_providers.insert(provider_guard.guarded_provider.identifier());
            }
        };
        if !result.is_success() {
            Self::pardon(punished_providers, provider_metrics.lock().unwrap());
        }

        result
    }

    fn select_provider<'a>(
        &self,
        provider_guards: &'a ProviderGuards<<<Self as Order>::J as Job>::P>,
        provider_metrics: &'a mut HashMap<ProviderIdentifier, ProviderMetrics>,
        custom_provider: &'a Option<<<Self as Order>::J as Job>::P>,
        exclude_providers: &HashSet<ProviderIdentifier>,
    ) -> (ProviderGuard<<<Self as Order>:: J as Job>::P>, bool) {
        match custom_provider {
            Some(p) => (ProviderGuard::new(p.clone()), true),
            None => {
                let (provider_guard, num_remaining) = if self.is_cacheable() {
                    provider_guards.get_provider_guard(|p, num_current_usages| {
                        if exclude_providers.contains(&p.identifier()) {
                            ProviderChoice::Exclude
                        } else {
                            let provider_metric = *(provider_metrics.get(&p.identifier()))
                                .unwrap_or(&ProviderMetrics::default());
                            let dynamic_metric = DynamicProviderMetrics {
                                num_failures: provider_metric.num_failures,
                                num_current_usages,
                                initial_score: p.initial_score(),
                            };
                            let score: <<Self as Order>:: J as Job>::DSC =
                                DynamicScoreCacheable::from_dynamic_provider_metrics(dynamic_metric);
                            ProviderChoice::Include(score)
                        }
                    })
                } else {
                    provider_guards.get_provider_guard(|p, num_current_usages| {
                        if exclude_providers.contains(&p.identifier()) {
                            ProviderChoice::Exclude
                        } else {
                            let provider_metric = *(provider_metrics.get(&p.identifier()))
                                .unwrap_or(&ProviderMetrics::default());
                            let dynamic_metric = DynamicProviderMetrics {
                                num_failures: provider_metric.num_failures,
                                num_current_usages,
                                initial_score: p.initial_score(),
                            };
                            let score: <<Self as Order>:: J as Job>::DSU =
                                DynamicScoreUncacheable::from_dynamic_provider_metrics(dynamic_metric);
                            ProviderChoice::Include(score)
                        }
                    })
                };
                debug!("Selected provider: {:?}", provider_guard);
                provider_metrics.entry(provider_guard.guarded_provider.identifier())
                    .and_modify(|e| {
                        e.num_usages += 1;
                    })
                    .or_insert(ProviderMetrics {
                        num_usages: 1,
                        num_failures: 0
                    });
                (provider_guard, num_remaining <= 1)
            }
        }
    }

    fn pardon(
        punished_providers: Vec<ProviderIdentifier>,
        mut provider_metrics: MutexGuard<HashMap<ProviderIdentifier, ProviderMetrics>>,
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

#[derive(PartialEq, Eq, PartialOrd, Clone, Copy, Debug)]
pub struct DynamicProviderMetrics<S> where S: Ord {
    pub num_failures: u32,
    pub num_current_usages: usize,
    pub initial_score: S,
}

/// A score used to compare providers when the order is cacheable.
pub trait DynamicScoreCacheable<S> : Ord where S: Ord {
    fn from_dynamic_provider_metrics(metrics: DynamicProviderMetrics<S>) -> Self;
}

/// A score used to compare providers when the order is uncacheable.
pub trait DynamicScoreUncacheable<S> : Ord where S: Ord {
    fn from_dynamic_provider_metrics(metrics: DynamicProviderMetrics<S>) -> Self;
}

pub trait Channel where Self: std::marker::Sized + std::fmt::Debug + std::marker::Send + 'static {
    type J: Job;

    fn progress_indicator(&self) -> Option<u64>;
    fn job_state(&mut self) -> &mut JobState<Self::J>;
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
    provider_guards: Arc<ProviderGuards<J::P>>,
    channels: Arc<Mutex<HashMap<J::P, J::C>>>,
    orders_in_progress: Arc<Mutex<HashSet<J::O>>>,
    provider_metrics: Arc<Mutex<HashMap<ProviderIdentifier, ProviderMetrics>>>,
    panic_monitor: Vec<Arc<Mutex<i32>>>,
    pub properties: J::PR,
}

#[derive(PartialEq, Eq, Hash, Clone, Debug, Copy, Default, Serialize)]
pub struct ProviderMetrics {
    pub num_usages: u32,
    pub num_failures: u32,
}

impl <J> JobContext<J> where J: Job {
    pub fn provider_metrics(&self) -> HashMap<ProviderIdentifier, ProviderMetrics> {
        return self.provider_metrics.lock().unwrap().clone();
    }

    pub fn reset_provider_metrics(&mut self) {
        self.provider_metrics.lock().unwrap().clear();
    }
}
pub struct ScheduledItem<J> where J: Job {
    pub join_handle: JoinHandle<JobOutcome<J>>,
    pub rx_integration_test: Receiver<IntegrationTestMessage>,
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
    Uncacheable(ProviderGuard<J::P>),
}

#[derive(PartialEq, Eq, Hash, Clone, Debug)]
/// Messages sent to monitor the state of Flexo during our integration tests.
pub enum IntegrationTestMessage {
    ProviderSelected(ProviderIdentifier),
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

#[derive(PartialEq, Eq, Hash, Clone, Debug, Copy)]
pub enum ChannelEstablishment {
    NewChannel,
    ExistingChannel,
}

impl <J> JobContext<J> where J: Job {
    pub fn new(initial_providers: Vec<J::P>, properties: J::PR) -> Self {
        Self::check_duplicates(&initial_providers);
        let provider_guards = Arc::new(ProviderGuards::new(initial_providers));
        let channels: Arc<Mutex<HashMap<J::P, J::C>>> = Arc::new(Mutex::new(HashMap::new()));
        let orders_in_progress: Arc<Mutex<HashSet<J::O>>> = Arc::new(Mutex::new(HashSet::new()));
        let provider_metrics: Arc<Mutex<HashMap<ProviderIdentifier, ProviderMetrics>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let thread_mutexes: Vec<Arc<Mutex<i32>>> = Vec::new();
        Self {
            provider_guards,
            channels,
            orders_in_progress,
            provider_metrics,
            panic_monitor: thread_mutexes,
            properties,
        }
    }

    fn check_duplicates(providers: &[J::P]) {
        let mut identifiers: HashSet<ProviderIdentifier> = HashSet::new();
        for p in providers.iter() {
            let identifier = p.identifier();
            if identifiers.contains(&identifier) {
                panic!("Identifiers must be unique: Got multiple occurrences of {}", identifier)
            }
            identifiers.insert(identifier);
        }
    }

    fn best_provider(&self, custom_provider: Option<J::P>) -> ProviderGuard<J::P> {
        // TODO this looks awkward.
        match custom_provider {
            None => {
                // no custom provider is required to fulfil this order: We can just choose the best provider
                // among all available providers.
                let (guard, _) = self.provider_guards.get_provider_guard(|g, _| {
                    ProviderChoice::Include(g.initial_score())
                });
                guard
            }
            Some(p) => {
                // This is a "special order" that needs to be served by a custom provider.
                // Speaking in Arch Linux terminology: This is a request that must be served
                // from a custom repository / unofficial repository.
                ProviderGuard::new(p)
            }
        }
    }

    /// Schedule the order, or return info on why scheduling this order is not possible or not necessary.
    pub fn try_schedule(
        &mut self,
        order: J::O,
        custom_provider: Option<J::P>,
        resume_from: Option<u64>,
    ) -> ScheduleOutcome<J>
        where <J as Job>::P: Sync
    {
        let resume_from = resume_from.unwrap_or(0);
        {
            let mut orders_in_progress = self.orders_in_progress.lock().unwrap();
            if orders_in_progress.contains(&order) {
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
                    None => {},
                    Some(CachedItem { cached_size, .. }) if cached_size < resume_from => {
                        // Cannot serve this order from cache: See issue #7
                        return ScheduleOutcome::Uncacheable(self.best_provider(custom_provider));
                    },
                    Some(CachedItem { complete_size: Some(c), cached_size }) if c == cached_size => {
                        debug!("Order {:?} is already cached.", &order);
                        return ScheduleOutcome::Cached;
                    },
                    Some(CachedItem { cached_size: _cached_size, .. }) => {}
                }
            }
            orders_in_progress.insert(order.clone());
        }
        self.schedule(order, custom_provider)
    }

    /// Schedules the job so that the order will be fetched from the provider.
    fn schedule(&mut self, order: J::O, custom_provider: Option<J::P>) -> ScheduleOutcome<J>
        where <J as Job>::P: Sync
    {
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

        let (tx_integration_test, rx_integration_test) = unbounded();
        let (tx_progress, rx_progress) = unbounded::<FlexoProgress>();
        let channels_cloned = Arc::clone(&self.channels);
        let mut provider_metrics_cloned = Arc::clone(&self.provider_metrics);
        let order_states = Arc::clone(&self.orders_in_progress);
        let provider_guards = Arc::clone(&self.provider_guards);
        let order_cloned = order.clone();
        let properties = self.properties.clone();

        let thread = thread::spawn(move || {
            let _lock = mutex_cloned.lock().unwrap();
            let order: <J as Job>::O = order.clone();
            let result = order.try_until_success(
                provider_guards,
                &mut provider_metrics_cloned,
                custom_provider,
                channels_cloned.clone(),
                tx_integration_test,
                tx_progress,
                properties,
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
                    let provider_metrics = provider_metrics_cloned.lock().unwrap().clone();
                    JobOutcome::Error(provider_metrics)
                }
                JobResult::Error(JobTerminated { mut channel, .. }) => {
                    channel.job_state().release_job_resources();
                    let provider_metrics = provider_metrics_cloned.lock().unwrap().clone();
                    JobOutcome::Error(provider_metrics)
                }
                JobResult::Unavailable(mut channel) => {
                    info!("{} was unavailable at all remote mirrors.", &order_cloned.description());
                    channel.job_state().release_job_resources();
                    let provider_metrics = provider_metrics_cloned.lock().unwrap().clone();
                    JobOutcome::Error(provider_metrics)
                }
                JobResult::ClientError => {
                    let provider_metrics = provider_metrics_cloned.lock().unwrap().clone();
                    JobOutcome::Error(provider_metrics)
                }
                JobResult::UnexpectedInternalError => {
                    let provider_metrics = provider_metrics_cloned.lock().unwrap().clone();
                    JobOutcome::Error(provider_metrics)
                }
            }
        });

        ScheduleOutcome::Scheduled(
            ScheduledItem {
                join_handle: thread,
                rx_integration_test,
                rx_progress,
            }
        )
    }
}

#[cfg(debug_assertions)]
fn send(message: IntegrationTestMessage, tx_integration_test: &Sender<IntegrationTestMessage>) {
    let _ = tx_integration_test.send(message);
}

#[cfg(not(debug_assertions))]
fn send(
    _message: IntegrationTestMessage,
    _tx_integration_test: &Sender<IntegrationTestMessage>
) where {
    // Nothing to do - no messages will be sent unless we're running tests.
}

