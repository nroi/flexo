extern crate http;
extern crate rand;

use flexo::*;
use std::collections::HashMap;
use crossbeam::channel::{Sender, Receiver};

static EXPECT_SCHEDULED: &str = "Expected the job to be scheduled";
static EXPECT_SKIPPED: &str = "Expected the job to be skipped";
static ORDER_PANIC: &str = "this order results in a panic!";
static EXPECT_SUCCESS: &str = "Expected the job to be completed successfully";
static EXPECT_FAILURE: &str = "Expected the job to fail, but it completed successfully";

#[derive(PartialEq, Eq, Hash, Clone, Debug)]
struct DummyProviderItem {
    identifier: i32,
    score: i32,
}

#[derive(PartialEq, Eq, Hash, Clone, Debug)]
enum DummyProvider {
    Success(DummyProviderItem),
    PartialCompletion(DummyProviderItem),
    Failure(DummyProviderItem),
}

#[derive(PartialEq, Eq, Hash, Clone, Debug)]
struct DummyState {
}

#[derive(PartialEq, Eq, Hash, Clone, Debug, Copy)]
struct DummyChannelState {}

impl Provider for DummyProvider {
    type J = DummyJob;

    fn new_job(&self, properties: &<<Self as Provider>::J as Job>::PR, order: DummyOrder) -> DummyJob {
        DummyJob {
            provider: self.clone(),
            order,
            properties: properties.clone(),
        }
    }

    fn initial_score(&self) -> i32 {
        match self {
            DummyProvider::Success(p) => p.score,
            DummyProvider::Failure(p) => p.score,
            DummyProvider::PartialCompletion(p) => p.score,
        }
    }

    fn description(&self) -> String {
        "DummyProvider".to_owned()
    }
}

#[derive(PartialEq, Eq, Hash, Clone, Debug)]
struct DummyJobError {
}

#[derive(PartialEq, Eq, Hash, Clone, Debug)]
struct DummyJob {
    provider: DummyProvider,
    order: DummyOrder,
    properties: DummyProperties,
}

#[derive(PartialEq, Eq, Hash, Copy, Clone, Debug)]
struct DummyProperties {}
impl Properties for DummyProperties {}

#[derive(PartialEq, Eq, Hash, Copy, Clone, Debug)]
struct DummyOrderError {}

impl Job for DummyJob {
    type S = i32;
    type JS = DummyState;
    type C = DummyChannel;
    type O = DummyOrder;
    type P = DummyProvider;
    type E = DummyJobError;
    type PI = i32;
    type PR = DummyProperties;
    type OE = DummyOrderError;

    fn provider(&self) -> &DummyProvider {
        &self.provider
    }

    fn order(&self) -> DummyOrder {
        self.order.clone()
    }

    fn properties(&self) -> Self::PR {
        self.properties
    }

    fn cache_state(_order: &Self::O, _properties: &Self::PR) -> Option<CachedItem> {
        None
    }

    fn serve_from_provider(self, channel: DummyChannel, _properties: &DummyProperties, _cached_size: u64) -> JobResult<DummyJob> {
        match (&self.order, &self.provider) {
            (DummyOrder::Success(_), DummyProvider::Success(_)) => {
                let jc = JobCompleted::new(channel, self.provider, 1);
                JobResult::Complete(jc)
            },
            (DummyOrder::Success(_), DummyProvider::PartialCompletion(_)) => {
                JobResult::Partial(JobPartiallyCompleted { channel, continue_at: 1 })
            },
            (DummyOrder::InfiniteBlocking(_), DummyProvider::Success(_)) => {
                let _result = channel.collector.tx.send(FlexoProgress::Progress(0));
                std::thread::park(); // block forever.
                JobResult::Complete(JobCompleted::new(channel, self.provider, 1))
            }
            (DummyOrder::Panic(_), _) => panic!("{}", ORDER_PANIC),
            _ => JobResult::Error(JobTerminated { channel, error: DummyJobError {} }),
        }
    }

    fn handle_error(self, _error: DummyOrderError) -> JobResult<Self> {
        unimplemented!()
    }

    fn acquire_resources(_order: &DummyOrder, _properties: &DummyProperties, _last_chance: bool) -> Result<DummyState, std::io::Error> {
        unimplemented!()
    }
}

#[derive(PartialEq, Eq, Hash, Clone, Debug, Copy)]
enum DummyOrder {
    /// an order which immediately completes successfully (unless the provider fails).
    Success(i32),
    /// an order which fails immediately.
    Failure(i32),
    /// an order which never finishes (unless the provider fails).
    InfiniteBlocking(i32),
    /// an order which results in a panic!
    Panic(i32),
}

impl Order for DummyOrder {
    type J = DummyJob;

    fn new_channel(self, _properties: <<Self as Order>::J as Job>::PR, tx: Sender<FlexoProgress>, _last_chance: bool) -> Result<DummyChannel, DummyOrderError> {
        Ok(DummyChannel {
            handle: 1,
            collector: JobState {
                order: self,
                job_resources: None,
                tx,
            },
            state: DummyChannelState {}
        })
    }

    fn reuse_channel(self, properties: <<Self as Order>::J as Job>::PR, tx: Sender<FlexoProgress>, last_chance: bool, _channel: DummyChannel) -> Result<DummyChannel, DummyOrderError> {
        self.new_channel(properties, tx, last_chance)
    }

    fn is_cacheable(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        "dummy description"
    }
}

#[derive(Debug)]
struct DummyChannel {
    handle: i32,
    collector: JobState<DummyJob>,
    state: DummyChannelState,
}

impl Channel for DummyChannel {
    type J = DummyJob;

    fn progress_indicator(&self) -> Option<u64> {
        Some(0)
    }

    fn job_state(&mut self) -> &mut JobState<DummyJob> {
        &mut self.collector
    }
}

struct DummyJobSuccess {
    provider: DummyProvider,
}

struct DummyJobFailure {
    metrics: HashMap<DummyProvider, ProviderMetrics>
}

fn successful_providers() -> Vec<DummyProvider> {
    vec![
        DummyProvider::Success(DummyProviderItem { identifier: 1, score: 1 }),
        DummyProvider::Success(DummyProviderItem { identifier: 2, score: 2 }),
        DummyProvider::Success(DummyProviderItem { identifier: 3, score: 3 }),
    ]
}

fn wait_until_message_received <F, R>(
    rx: Receiver<FlexoMessage<DummyProvider>>,
    message_cmp: F
) -> R where F: Fn(&FlexoMessage<DummyProvider>) -> Option<R> {
    match rx.recv().unwrap() {
        received_message => {
            match message_cmp(&received_message) {
                Some(result) => result,
                None => wait_until_message_received(rx, message_cmp),
            }
        }
    }
}

fn wait_until_provider_selected(schedule_outcome: ScheduleOutcome<DummyJob>) -> DummyProvider {
    match schedule_outcome {
        ScheduleOutcome::Scheduled(ScheduledItem { join_handle: _, rx, rx_progress: _ }) => {
            let message_cmp = |msg: &FlexoMessage<DummyProvider>| {
                match msg {
                    FlexoMessage::ProviderSelected(p) => Some(p.clone()),
                    _ => None
                }
            };
            wait_until_message_received(rx, message_cmp)
        },
        _ => panic!("{}", EXPECT_SCHEDULED),
    }
}

fn wait_until_channel_established(schedule_outcome: ScheduleOutcome<DummyJob>) {
    match schedule_outcome {
        ScheduleOutcome::Scheduled(ScheduledItem { join_handle: _, rx, rx_progress: _  }) => {
            let message_cmp = |msg: &FlexoMessage<DummyProvider>| {
                match msg {
                    FlexoMessage::ChannelEstablished(_) => Some(true),
                    _ => None,
                }
            };
            wait_until_message_received(rx, message_cmp);
        },
        _ => panic!("{}", EXPECT_SCHEDULED),
    };
}

fn wait_until_job_completed(schedule_outcome: ScheduleOutcome<DummyJob>) -> DummyJobSuccess {
    let result = match schedule_outcome {
        ScheduleOutcome::Scheduled(ScheduledItem { join_handle, rx: _, rx_progress: _  }) => {
            join_handle.join().unwrap()
        },
        _ => panic!("{}", EXPECT_SCHEDULED),
    };
    match result {
        JobOutcome::Success(provider) => DummyJobSuccess { provider },
        JobOutcome::Error(_) => panic!("{}", EXPECT_SUCCESS),
    }
}

fn wait_until_job_failed(schedule_outcome: ScheduleOutcome<DummyJob>) -> DummyJobFailure {
    let result = match schedule_outcome {
        ScheduleOutcome::Scheduled(ScheduledItem { join_handle, rx: _, rx_progress: _  }) => {
            join_handle.join().unwrap()
        },
        _ => panic!("{}", EXPECT_SCHEDULED),
    };
    match result {
        JobOutcome::Success(_) => panic!("{}", EXPECT_FAILURE),
        JobOutcome::Error(failures) => DummyJobFailure {
            metrics: failures,
        }
    }
}

#[test]
fn provider_lowest_score() {
    // Given more than one available provider, the provider with the lowest score is selected.
    let p1 = DummyProvider::Success(DummyProviderItem { identifier: 1, score: 0 });
    let p2 = DummyProvider::Success(DummyProviderItem { identifier: 1, score: 1 });
    let providers = vec![p1.clone(), p2.clone()];
    let mut job_context: JobContext<DummyJob> = JobContext::new(providers, DummyProperties{});
    let result = match job_context.try_schedule(DummyOrder::Success(0), None, None) {
        ScheduleOutcome::Scheduled(ScheduledItem { join_handle, rx: _, rx_progress: _ }) => {
            // wait for the job to complete.
            join_handle.join()
        },
        _ => panic!("{}", EXPECT_SCHEDULED),
    };
    result.unwrap();
}

#[test]
fn second_provider_success_after_first_provider_failure() {
    let p1 = DummyProvider::Failure(DummyProviderItem { identifier: 1, score: 0 });
    let p2 = DummyProvider::Success(DummyProviderItem { identifier: 1, score: 1 });
    let providers = vec![p1.clone(), p2.clone()];
    let mut job_context: JobContext<DummyJob> = JobContext::new(providers, DummyProperties{});
    match job_context.try_schedule(DummyOrder::Success(0), None, None) {
        ScheduleOutcome::Scheduled(ScheduledItem { join_handle, rx: _, rx_progress: _ }) => {
            // wait for the job to complete.
            join_handle.join().unwrap();
        },
        _ => panic!("{}", EXPECT_SCHEDULED),
    }
    let result = job_context.try_schedule(DummyOrder::Success(1), None, None);
    let DummyJobSuccess { provider } = wait_until_job_completed(result);
    assert_eq!(provider, p2);
}

#[test]
fn next_order_success_after_first_order_failed() {
    // After a first order has failed, a subsequent order succeeds: this test case is intended to ensure
    // that a failing job does not cause all available providers to be "blacklisted", i.e., when some mechanism
    // is used to downgrade a provider after it has failed to complete an order, a subsequent order should still
    // be able to use this provider, even though it has been downgraded.
    let mut job_context: JobContext<DummyJob> = JobContext::new(successful_providers(), DummyProperties{});
    job_context.try_schedule(DummyOrder::Failure(0), None, None);
    match job_context.try_schedule(DummyOrder::Success(1), None, None) {
        ScheduleOutcome::Scheduled(ScheduledItem { join_handle, rx: _, rx_progress: _ }) => {
            let result = join_handle.join().unwrap();
            match result {
                JobOutcome::Success(_) => {},
                _ => panic!("Expected success"),
            }
        },
        _ => assert!(false, "{}", EXPECT_SCHEDULED),
    };
}

#[test]
fn provider_no_two_simultaneous_jobs() {
    // Once an order has been assigned to a provider, this provider will not be used again as long as the job
    // is still in progress. The intention is to reduce load on the provider by not running multiple jobs
    // simultaneously (or, to speak in more specific terms: we don't want to strain the same web server with more
    // more than one download).
    let p1 = DummyProvider::Success(DummyProviderItem { identifier: 1, score: 0 });
    let p2 = DummyProvider::Success(DummyProviderItem { identifier: 1, score: 1 });
    let providers = vec![p1.clone(), p2.clone()];
    let mut job_context: JobContext<DummyJob> = JobContext::new(providers, DummyProperties{});
    let provider_order1 = match job_context.try_schedule(DummyOrder::InfiniteBlocking(0), None, None) {
        ScheduleOutcome::Scheduled(ScheduledItem { join_handle: _, rx, rx_progress: _ }) => {
            rx.recv().unwrap()
        },
        _ => panic!("{}", EXPECT_SCHEDULED),
    };
    let provider_order2 = match job_context.try_schedule(DummyOrder::Success(1), None, None) {
        ScheduleOutcome::Scheduled(ScheduledItem { join_handle: _, rx, rx_progress: _ }) => {
            rx.recv().unwrap()
        },
        _ => panic!("{}", EXPECT_SCHEDULED),
    };
    assert_ne!(provider_order1, provider_order2);
}

#[test]
fn provider_two_simultaneous_jobs_if_required() {
    // While we generally want to avoid to have one provider handling more than one job simultaneously, this can be
    // necessary if the number of providers is low and the frequency of newly arriving orders is high.
    let p1 = DummyProvider::Success(DummyProviderItem { identifier: 1, score: 0 });
    let providers = vec![p1.clone()];
    let mut job_context: JobContext<DummyJob> = JobContext::new(providers, DummyProperties{});
    let provider_order1 = match job_context.try_schedule(DummyOrder::InfiniteBlocking(0), None, None) {
        ScheduleOutcome::Scheduled(ScheduledItem { join_handle: _, rx, rx_progress: _ }) => {
            rx.recv().unwrap()
        },
        _ => panic!("{}", EXPECT_SCHEDULED),
    };
    let provider_order2 = match job_context.try_schedule(DummyOrder::Success(1), None, None) {
        ScheduleOutcome::Scheduled(ScheduledItem { join_handle: _, rx, rx_progress: _ }) => {
            rx.recv().unwrap()
        },
        _ => panic!("{}", EXPECT_SCHEDULED),
    };
    assert_eq!(provider_order1, provider_order2);
}

#[test]
fn order_skipped_if_already_in_progress() {
    // If an order is already in progress, scheduling the same order again will not cause a new job to be
    // executed. The intention here is that, since the library will be used for downloads, we don't ever want to
    // have two simultaneous downloads of the same file in order to conserve bandwidth.
    let p1 = DummyProvider::Success(DummyProviderItem { identifier: 1, score: 0 });
    let order = DummyOrder::InfiniteBlocking(0);
    let providers = vec![p1.clone()];
    let mut job_context: JobContext<DummyJob> = JobContext::new(providers, DummyProperties{});
    wait_until_provider_selected(job_context.try_schedule(order.clone(), None, None));

    match job_context.try_schedule(order.clone(), None, None) {
        ScheduleOutcome::AlreadyInProgress =>
            {}
        ScheduleOutcome::Scheduled(_) =>
            panic!("{}", EXPECT_SKIPPED),
        ScheduleOutcome::Cached =>
            panic!("{}", EXPECT_SKIPPED),
        ScheduleOutcome::Uncacheable(_) =>
            panic!("{}", EXPECT_SKIPPED),
    }
}

#[test]
fn best_provider_selected() {
    // Given many providers with different scores: If no failures have occurred yet, and no providers are
    // currently in use, the provider with the best score (i.e., the lowest score) is selected.
    let p1 = DummyProvider::Success(DummyProviderItem { identifier: 1, score: 1 });
    let p2 = DummyProvider::Success(DummyProviderItem { identifier: 2, score: -1 });
    let p3 = DummyProvider::Success(DummyProviderItem { identifier: 3, score: 2 });
    let providers = vec![p1.clone(), p2.clone(), p3.clone()];
    let mut job_context: JobContext<DummyJob> = JobContext::new(providers, DummyProperties{});
    let result = job_context.try_schedule(DummyOrder::Success(0), None, None);

    let DummyJobSuccess { provider } = wait_until_job_completed(result);
    assert_eq!(provider, p2);
}

#[test]
fn job_continued_after_partial_completion() {
    // If a job could be only partially completed, it does not need to be restarted from scratch.
    let p1 = DummyProvider::PartialCompletion(DummyProviderItem { identifier: 1, score: 1 });
    let p2 = DummyProvider::Success(DummyProviderItem { identifier: 2, score: 2 });
    let p3 = DummyProvider::Success(DummyProviderItem { identifier: 2, score: 3 });
    let providers = vec![p1.clone(), p2.clone(), p3.clone()];
    let mut job_context: JobContext<DummyJob> = JobContext::new(providers, DummyProperties{});
    let (provider_first_scheduled, provider_finally_scheduled) = match job_context.try_schedule(DummyOrder::Success(0), None, None) {
        ScheduleOutcome::Scheduled(ScheduledItem {join_handle, rx, rx_progress: _ }) => {
            let provider_first_scheduled = match rx.recv().unwrap() {
                FlexoMessage::ProviderSelected(p) => p,
                _ => panic!("Did not expect this message")
            };
            let provider_finally_scheduled = match join_handle.join().unwrap() {
                JobOutcome::Success(p) => p,
                JobOutcome::Error(_) => panic!("Expected success"),
            };
            (provider_first_scheduled, provider_finally_scheduled)
        },
        _ => panic!("{}", EXPECT_SCHEDULED),
    };
    assert_eq!(provider_first_scheduled, p1);
    assert_eq!(provider_finally_scheduled, p2);
}

#[test]
fn no_infinite_loop() {
    // if all providers fail to fulfil the order, no infinite loop results.
    let p1 = DummyProvider::Failure(DummyProviderItem { identifier: 1, score: 1 });
    let providers = vec![p1.clone()];
    let mut job_context: JobContext<DummyJob> = JobContext::new(providers, DummyProperties{});
    let result = match job_context.try_schedule(DummyOrder::Success(0), None, None) {
        ScheduleOutcome::Scheduled(ScheduledItem {join_handle, rx: _, rx_progress: _ }) => {
            join_handle.join().unwrap()
        },
        _ => panic!("{}", EXPECT_SCHEDULED),
    };

    match result {
        JobOutcome::Success(_) => panic!("{}", EXPECT_SUCCESS),
        JobOutcome::Error(_) => {},
    }
}

#[test]
fn downgrade_provider() {
    // We have two providers p1 and p2 available, where p1 has the better score: In the first run,
    // p1 will be selected, and this provider fails, resulting in the provider being downgraded.
    // For the subsequently scheduled order, p2 will be selected, even though its score is worse than p1:
    // this is due to the fact that p1 has been downgraded after the failure has occurred.
    let p1 = DummyProvider::Failure(DummyProviderItem { identifier: 1, score: 1 });
    let p2 = DummyProvider::Success(DummyProviderItem { identifier: 2, score: 2 });
    let providers = vec![p1.clone(), p2.clone()];
    let mut job_context: JobContext<DummyJob> = JobContext::new(providers, DummyProperties{});
    let result1 = job_context.try_schedule(DummyOrder::Success(0), None, None);
    wait_until_job_completed(result1);
    let result2 = job_context.try_schedule(DummyOrder::Success(1), None, None);
    let first_provider_selected = wait_until_provider_selected(result2);
    assert_eq!(first_provider_selected, p2);
}

#[test]
fn no_downgrade_if_all_providers_fail() {
    // Consider the case when a job cannot be completed because the file simply does not exist, or because we
    // don't have a network connection. We don't want a provider to be downgraded.
    // Therefore, we generally want to downgrade a provider only if it failed to fulfil the order while another
    // provider was able to fulfil it, since this is a strong indication that the provider is the culprit, not
    // the client or the order.
    let p1 = DummyProvider::Failure(DummyProviderItem { identifier: 1, score: 1 });
    let providers = vec![p1.clone()];
    let mut job_context: JobContext<DummyJob> = JobContext::new(providers, DummyProperties{});
    let result1 = job_context.try_schedule(DummyOrder::Success(0), None, None);
    let DummyJobFailure { metrics } = wait_until_job_failed(result1);
    let metrics = metrics.get(&p1);
    match metrics {
        Some(ProviderMetrics { num_failures: 0, .. }) => {}
        e => panic!("Expected a metric with no failures, got instead: {:?}", e)
    }
}

#[test]
fn no_new_channel_established() {
    // channels can be reused: If a job has completed, the channel used for this job will be retained such that
    // it can be reused by a subsequent job.
    let p1 = DummyProvider::Success(DummyProviderItem { identifier: 1, score: 1 });
    let providers = vec![p1.clone()];
    let mut job_context: JobContext<DummyJob> = JobContext::new(providers, DummyProperties{});
    let result1 = job_context.try_schedule(DummyOrder::Success(0), None, None);
    wait_until_job_completed(result1);
    let channel_establishment = match job_context.try_schedule(DummyOrder::Success(1), None, None) {
        ScheduleOutcome::Scheduled(p) => {
            wait_until_message_received(p.rx, |msg| {
                match msg {
                    FlexoMessage::ChannelEstablished(c) => Some(*c),
                    _ => None,
                }
            })
        },
        _ => panic!("{}", EXPECT_SCHEDULED),
    };
    assert_eq!(channel_establishment, ChannelEstablishment::ExistingChannel)
}

#[test]
fn new_channel_established_because_channel_in_use() {
    // A channel can only be used for one job at any given time. If the job is still in progress,
    // we cannot reuse the existing channel, therefore, a new channel must be established.
    let p1 = DummyProvider::Success(DummyProviderItem { identifier: 1, score: 1 });
    let providers = vec![p1.clone()];
    let mut job_context: JobContext<DummyJob> = JobContext::new(providers, DummyProperties{});
    let result1 = job_context.try_schedule(DummyOrder::InfiniteBlocking(0), None, None);
    wait_until_channel_established(result1);
    let channel_establishment = match job_context.try_schedule(DummyOrder::Success(1), None, None) {
        ScheduleOutcome::Scheduled(p) => {
            wait_until_message_received(p.rx, |msg| {
                match msg {
                    FlexoMessage::ProviderSelected(_) => None,
                    FlexoMessage::OrderError => None,
                    FlexoMessage::ChannelEstablished(c) => Some(*c),
                }
            })
        },
        _ => panic!("{}", EXPECT_SCHEDULED),
    };
    assert_eq!(channel_establishment, ChannelEstablishment::NewChannel)
}

#[test]
#[should_panic]
fn job_panic_results_in_main_panic() {
    // If an order fails with a panic!, attempting to schedule a second order results in a panic.
    // This is due to the fact that panics stop the thread's execution, so we don't want to continue running the
    // program since we require this thread to met various invariants. At the same time, Rust does not provide an
    // easy way (afaik) that allows a thread panic to propagate to the parent thread, so we cannot just panic! the
    // main thread when a child thread completes. But we can detect a panic of a child thread before we schedule
    // a new job.
    let p1 = DummyProvider::Success(DummyProviderItem { identifier: 1, score: 1 });
    let order1 = DummyOrder::Panic(0);
    let order2 = DummyOrder::Success(1);
    let providers = vec![p1.clone()];
    let mut job_context: JobContext<DummyJob> = JobContext::new(providers, DummyProperties{});
    let result1 = job_context.try_schedule(order1, None, None);
    wait_until_job_failed(result1);
    job_context.try_schedule(order2, None, None);
}

#[test]
fn read_progress() {
    // the rx_progress channel can be used to inform the caller about progress being made before the job
    // has finished.
    let p1 = DummyProvider::Success(DummyProviderItem { identifier: 1, score: 0 });
    let providers = vec![p1.clone()];
    let mut job_context: JobContext<DummyJob> = JobContext::new(providers, DummyProperties{});
    let result = match job_context.try_schedule(DummyOrder::InfiniteBlocking(0), None, None) {
        ScheduleOutcome::Scheduled(ScheduledItem { join_handle: _, rx: _, rx_progress }) => {
            rx_progress.recv_timeout(std::time::Duration::from_millis(50)).unwrap()
        },
        _ => panic!("{}", EXPECT_SCHEDULED),
    };
    assert_eq!(result, FlexoProgress::Progress(0));
}
