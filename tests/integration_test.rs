#![feature(vec_remove_item)]
#![feature(with_options)]

extern crate http;
extern crate rand;

use flexo::*;
use std::sync::mpsc::Receiver;
use std::collections::HashMap;

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
struct DummyChannelState {
    pub is_reset: bool,
}

impl ChannelState for DummyChannelState {
    type J = DummyJob;

    fn reset(&mut self) {
        self.is_reset = true;
    }
}

impl JobState for DummyState {}

impl Provider for DummyProvider {
    type J = DummyJob;

    fn new_job(&self, order: DummyOrder) -> DummyJob {
        DummyJob {
            provider: self.clone(),
            order,
        }
    }

    fn identifier(&self) -> &i32 {
        match self {
            DummyProvider::Success(DummyProviderItem { identifier, score: _ }) => identifier,
            DummyProvider::Failure(DummyProviderItem { identifier, score: _ }) => identifier,
            DummyProvider::PartialCompletion(DummyProviderItem { identifier, score: _ }) => identifier,
        }
    }

    fn score(&self) -> i32 {
        match self {
            DummyProvider::Success(p) => p.score,
            DummyProvider::Failure(p) => p.score,
            DummyProvider::PartialCompletion(p) => p.score,
        }
    }
}

#[derive(PartialEq, Eq, Hash, Clone, Debug)]
struct DummyJobError {
}

#[derive(PartialEq, Eq, Hash, Clone, Debug)]
struct DummyJob {
    provider: DummyProvider,
    order: DummyOrder,
}

impl Job for DummyJob {
    type S = i32;
    type JS = DummyState;
    type C = DummyChannel;
    type O = DummyOrder;
    type P = DummyProvider;
    type E = DummyJobError;
    type CS = DummyChannelState;
    type PI = i32;

    fn provider(&self) -> &DummyProvider {
        &self.provider
    }

    fn order(&self) -> DummyOrder {
        self.order.clone()
    }

    fn execute(self, channel: DummyChannel) -> JobResult<DummyJob> {
        match (&self.order, &self.provider) {
            (DummyOrder::Success(_), DummyProvider::Success(_)) =>
                JobResult::Complete(JobCompleted::new(channel, self.provider)),
            (DummyOrder::Success(_), DummyProvider::PartialCompletion(_)) => {
                JobResult::Partial(JobPartiallyCompleted { channel, continue_at: 1 })
            },
            (DummyOrder::InfiniteBlocking(_), DummyProvider::Success(_)) => {
                std::thread::park(); // block forever.
                JobResult::Complete(JobCompleted::new(channel, self.provider))
            }
            (DummyOrder::Panic(_), _) => panic!(ORDER_PANIC),
            _ => JobResult::Error(JobTerminated { channel, error: DummyJobError {} }),
        }
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

    fn new_channel(self) -> DummyChannel {
        DummyChannel {
            handle: 1,
            collector: JobStateItem {
                order: self,
                state: None,
            },
            state: DummyChannelState {
                is_reset: false,
            }
        }
    }
}

#[derive(Debug)]
struct DummyChannel {
    handle: i32,
    collector: JobStateItem<DummyJob>,
    state: DummyChannelState,
}

impl Channel for DummyChannel {
    type J = DummyJob;

    fn progress_indicator(&self) -> Option<u64> {
        // TODO
        Some(5)
    }

    fn reset_order(&mut self, _order: DummyOrder) {
        // TODO
    }

    fn channel_state_item(&mut self) -> &mut JobStateItem<DummyJob> {
        &mut self.collector
    }

    fn channel_state(&self) -> DummyChannelState {
        self.state
    }

    fn channel_state_ref(&mut self) -> &mut DummyChannelState {
        &mut self.state
    }
}

struct DummyJobSuccess {
    provider: DummyProvider,
    state: DummyChannelState,
}

struct DummyJobFailure {
    channel_state: DummyChannelState,
    failures: HashMap<DummyProvider, i32>
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

fn wait_until_provider_selected(schedule_outcome: ScheduleOutcome<DummyProvider, DummyChannelState>) -> DummyProvider {
    match schedule_outcome {
        ScheduleOutcome::Skipped => panic!(EXPECT_SCHEDULED),
        ScheduleOutcome::Scheduled(ScheduledItem { join_handle: _, rx }) => {
            let message_cmp = |msg: &FlexoMessage<DummyProvider>| {
                match msg {
                    FlexoMessage::ProviderSelected(p) => Some(p.clone()),
                    _ => None
                }
            };
            wait_until_message_received(rx, message_cmp)
        }
    }
}

fn wait_until_channel_established(schedule_outcome: ScheduleOutcome<DummyProvider, DummyChannelState>) {
    match schedule_outcome {
        ScheduleOutcome::Skipped => panic!(EXPECT_SCHEDULED),
        ScheduleOutcome::Scheduled(ScheduledItem { join_handle: _, rx }) => {
            let message_cmp = |msg: &FlexoMessage<DummyProvider>| {
                match msg {
                    FlexoMessage::ChannelEstablished(_) => Some(true),
                    _ => None,
                }
            };
            wait_until_message_received(rx, message_cmp);
        }
    };
}

fn wait_until_job_completed(schedule_outcome: ScheduleOutcome<DummyProvider, DummyChannelState>) -> DummyJobSuccess {
    let result = match schedule_outcome {
        ScheduleOutcome::Skipped => panic!(EXPECT_SCHEDULED),
        ScheduleOutcome::Scheduled(ScheduledItem { join_handle, rx: _ }) => {
            join_handle.join().unwrap()
        }
    };
    match result {
        JobOutcome::Success(provider, state) => DummyJobSuccess { provider, state},
        JobOutcome::Error(_, _) => panic!(EXPECT_SUCCESS),
    }
}

fn wait_until_job_failed(schedule_outcome: ScheduleOutcome<DummyProvider, DummyChannelState>) -> DummyJobFailure {
    let result = match schedule_outcome {
        ScheduleOutcome::Skipped => panic!(EXPECT_SCHEDULED),
        ScheduleOutcome::Scheduled(ScheduledItem { join_handle, rx: _ }) => {
            join_handle.join().unwrap()
        }
    };
    match result {
        JobOutcome::Success(_, _) => panic!(EXPECT_FAILURE),
        JobOutcome::Error(failures, channel_state) => DummyJobFailure {
            failures,
            channel_state,
        }
    }
}

#[test]
fn provider_lowest_score() {
    // Given more than one available provider, the provider with the lowest score is selected.
    let p1 = DummyProvider::Success(DummyProviderItem { identifier: 1, score: 0 });
    let p2 = DummyProvider::Success(DummyProviderItem { identifier: 1, score: 1 });
    let providers = vec![p1.clone(), p2.clone()];
    let mut job_context: JobContext<DummyJob> = JobContext::new(providers);
    let result = match job_context.schedule(DummyOrder::Success(0)) {
        ScheduleOutcome::Skipped => panic!(EXPECT_SCHEDULED),
        ScheduleOutcome::Scheduled(ScheduledItem { join_handle, rx: _ }) => {
            // wait for the job to complete.
            join_handle.join()
        },
    };
    result.unwrap();
}

#[test]
fn second_provider_success_after_first_provider_failure() {
    let p1 = DummyProvider::Failure(DummyProviderItem { identifier: 1, score: 0 });
    let p2 = DummyProvider::Success(DummyProviderItem { identifier: 1, score: 1 });
    let providers = vec![p1.clone(), p2.clone()];
    let mut job_context: JobContext<DummyJob> = JobContext::new(providers);
    match job_context.schedule(DummyOrder::Success(0)) {
        ScheduleOutcome::Skipped => panic!(EXPECT_SCHEDULED),
        ScheduleOutcome::Scheduled(ScheduledItem { join_handle, rx: _ }) => {
            // wait for the job to complete.
            join_handle.join().unwrap();
        },
    }
    let result = job_context.schedule(DummyOrder::Success(1));
    let DummyJobSuccess { provider, state: _ } = wait_until_job_completed(result);
    assert_eq!(provider, p2);
}

#[test]
fn next_order_success_after_first_order_failed() {
    // After a first order has failed, a subsequent order succeeds: this test case is intended to ensure
    // that a failing job does not cause all available providers to be "blacklisted", i.e., when some mechanism
    // is used to downgrade a provider after it has failed to complete an order, a subsequent order should still
    // be able to use this provider, even though it has been downgraded.
    let mut job_context: JobContext<DummyJob> = JobContext::new(successful_providers());
    job_context.schedule(DummyOrder::Failure(0));
    match job_context.schedule(DummyOrder::Success(1)) {
        ScheduleOutcome::Skipped => assert!(false, EXPECT_SCHEDULED),
        ScheduleOutcome::Scheduled(ScheduledItem { join_handle, rx: _ }) => {
            let result = join_handle.join().unwrap();
            match result {
                JobOutcome::Success(_, _) => {},
                _ => panic!("Expected success"),
            }
        },
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
    let mut job_context: JobContext<DummyJob> = JobContext::new(providers);
    let provider_order1 = match job_context.schedule(DummyOrder::InfiniteBlocking(0)) {
        ScheduleOutcome::Skipped => panic!(EXPECT_SCHEDULED),
        ScheduleOutcome::Scheduled(ScheduledItem { join_handle: _, rx }) => {
            rx.recv().unwrap()
        }
    };
    let provider_order2 = match job_context.schedule(DummyOrder::Success(1)) {
        ScheduleOutcome::Skipped => panic!(EXPECT_SCHEDULED),
        ScheduleOutcome::Scheduled(ScheduledItem { join_handle: _, rx }) => {
            rx.recv().unwrap()
        }
    };
    assert_ne!(provider_order1, provider_order2);
}

#[test]
fn provider_two_simultaneous_jobs_if_required() {
    // While we generally want to avoid to have one provider handling more than one job simultaneously, this can be
    // necessary if the number of providers is low and the frequency of newly arriving jobs is high.
    let p1 = DummyProvider::Success(DummyProviderItem { identifier: 1, score: 0 });
    let providers = vec![p1.clone()];
    let mut job_context: JobContext<DummyJob> = JobContext::new(providers);
    let provider_order1 = match job_context.schedule(DummyOrder::InfiniteBlocking(0)) {
        ScheduleOutcome::Skipped => panic!(EXPECT_SCHEDULED),
        ScheduleOutcome::Scheduled(ScheduledItem { join_handle: _, rx }) => {
            rx.recv().unwrap()
        }
    };
    let provider_order2 = match job_context.schedule(DummyOrder::Success(1)) {
        ScheduleOutcome::Skipped => panic!(EXPECT_SCHEDULED),
        ScheduleOutcome::Scheduled(ScheduledItem { join_handle: _, rx }) => {
            rx.recv().unwrap()
        }
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
    let mut job_context: JobContext<DummyJob> = JobContext::new(providers);
    wait_until_provider_selected(job_context.schedule(order.clone()));
    match job_context.schedule(order.clone()) {
        ScheduleOutcome::Skipped => {},
        ScheduleOutcome::Scheduled(_) => panic!(EXPECT_SKIPPED)
    }
}


#[test]
fn order_not_skipped_after_completed() {
    // Orders should only be skipped if they are currently in progress: If, on the other hand, they have already
    // been completed, we can schedule new orders.
    let p1 = DummyProvider::Success(DummyProviderItem { identifier: 1, score: 0 });
    let order = DummyOrder::Success(0);
    let providers = vec![p1.clone()];
    let mut job_context: JobContext<DummyJob> = JobContext::new(providers);
    wait_until_job_completed(job_context.schedule(order.clone()));
    match job_context.schedule(order.clone()) {
        ScheduleOutcome::Skipped => panic!(EXPECT_SCHEDULED),
        ScheduleOutcome::Scheduled(_) => {}
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
    let mut job_context: JobContext<DummyJob> = JobContext::new(providers);
    let result = job_context.schedule(DummyOrder::Success(0));

    let DummyJobSuccess { provider, state: _ } = wait_until_job_completed(result);
    assert_eq!(provider, p2);
}

#[test]
fn job_continued_after_partial_completion() {
    // If a job could be only partially completed, it does not need to be restarted from scratch.
    let p1 = DummyProvider::PartialCompletion(DummyProviderItem { identifier: 1, score: 1 });
    let p2 = DummyProvider::Success(DummyProviderItem { identifier: 2, score: 2 });
    let p3 = DummyProvider::Success(DummyProviderItem { identifier: 2, score: 3 });
    let providers = vec![p1.clone(), p2.clone(), p3.clone()];
    let mut job_context: JobContext<DummyJob> = JobContext::new(providers);
    let (provider_first_scheduled, provider_finally_scheduled) = match job_context.schedule(DummyOrder::Success(0)) {
        ScheduleOutcome::Skipped => panic!(EXPECT_SCHEDULED),
        ScheduleOutcome::Scheduled(ScheduledItem {join_handle, rx}) => {
            let provider_first_scheduled = match rx.recv().unwrap() {
                FlexoMessage::ProviderSelected(p) => p,
                FlexoMessage::ChannelEstablished(_) => panic!("Did not expect this message")
            };
            let provider_finally_scheduled = match join_handle.join().unwrap() {
                JobOutcome::Success(p, _) => p,
                JobOutcome::Error(_, _) => panic!("Expected success"),
            };
            (provider_first_scheduled, provider_finally_scheduled)
        }
    };
    assert_eq!(provider_first_scheduled, p1);
    assert_eq!(provider_finally_scheduled, p2);
}

#[test]
fn no_infinite_loop() {
    // if all providers fail to fulfil the order, no infinite loop results.
    let p1 = DummyProvider::Failure(DummyProviderItem { identifier: 1, score: 1 });
    let providers = vec![p1.clone()];
    let mut job_context: JobContext<DummyJob> = JobContext::new(providers);
    let result = match job_context.schedule(DummyOrder::Success(0)) {
        ScheduleOutcome::Skipped => panic!(EXPECT_SCHEDULED),
        ScheduleOutcome::Scheduled(ScheduledItem {join_handle, rx: _}) => {
            join_handle.join().unwrap()
        }
    };

    match result {
        JobOutcome::Success(_, _) => panic!(EXPECT_SUCCESS),
        JobOutcome::Error(_, _) => {},
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
    let mut job_context: JobContext<DummyJob> = JobContext::new(providers);
    let result1 = job_context.schedule(DummyOrder::Success(0));
    wait_until_job_completed(result1);
    let result2 = job_context.schedule(DummyOrder::Success(1));
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
    let mut job_context: JobContext<DummyJob> = JobContext::new(providers);
    let result1 = job_context.schedule(DummyOrder::Success(0));
    let DummyJobFailure { channel_state: _, failures } = wait_until_job_failed(result1);
    let failures = failures.get(&p1);
    assert_eq!(failures, Some(&0));
}

#[test]
fn no_new_channel_established() {
    // channels can be reused: If a job has completed, the channel used for this job will be retained such that
    // it can be reused by a subsequent job.
    let p1 = DummyProvider::Success(DummyProviderItem { identifier: 1, score: 1 });
    let providers = vec![p1.clone()];
    let mut job_context: JobContext<DummyJob> = JobContext::new(providers);
    let result1 = job_context.schedule(DummyOrder::Success(0));
    wait_until_job_completed(result1);
    let channel_establishment = match job_context.schedule(DummyOrder::Success(1)) {
        ScheduleOutcome::Skipped => panic!(EXPECT_SCHEDULED),
        ScheduleOutcome::Scheduled(p) => {
            wait_until_message_received(p.rx, |msg| {
                match msg {
                    FlexoMessage::ProviderSelected(_) => None,
                    FlexoMessage::ChannelEstablished(c) => Some(*c),
                }
            })
        },
    };
    assert_eq!(channel_establishment, ChannelEstablishment::ExistingChannel)
}

#[test]
fn new_channel_established_because_channel_in_use() {
    // A channel can only be used for one job at any given time. If the job is still in progress,
    // we cannot reuse the existing channel, therefore, a new channel must be established.
    let p1 = DummyProvider::Success(DummyProviderItem { identifier: 1, score: 1 });
    let providers = vec![p1.clone()];
    let mut job_context: JobContext<DummyJob> = JobContext::new(providers);
    let result1 = job_context.schedule(DummyOrder::InfiniteBlocking(0));
    wait_until_channel_established(result1);
    let channel_establishment = match job_context.schedule(DummyOrder::Success(1)) {
        ScheduleOutcome::Skipped => panic!(EXPECT_SCHEDULED),
        ScheduleOutcome::Scheduled(p) => {
            wait_until_message_received(p.rx, |msg| {
                match msg {
                    FlexoMessage::ProviderSelected(_) => None,
                    FlexoMessage::ChannelEstablished(c) => Some(*c),
                }
            })
        },
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
    let mut job_context: JobContext<DummyJob> = JobContext::new(providers);
    let result1 = job_context.schedule(order1);
    wait_until_job_failed(result1);
    job_context.schedule(order2);
}

#[test]
fn reset_after_success() {
    // To ensure that all resources acquired for a given job are closed, a Channel is associated with a
    // ChannelStateItem. The ChannelStateItem should be dropped in order to close all resources that are
    // no longer required.
    let p1 = DummyProvider::Success(DummyProviderItem { identifier: 1, score: 1 });
    let order1 = DummyOrder::Success(0);
    let providers = vec![p1.clone()];
    let mut job_context: JobContext<DummyJob> = JobContext::new(providers);
    let DummyJobSuccess { provider: _, state } = wait_until_job_completed(job_context.schedule(order1));
    assert_eq!(state.is_reset, true);
}

#[test]
fn reset_after_failure() {
    // Same as reset_after_success, the resources are not required anymore and should therefore be released.
    let p1 = DummyProvider::Success(DummyProviderItem { identifier: 1, score: 1 });
    let order1 = DummyOrder::Failure(0);
    let providers = vec![p1.clone()];
    let mut job_context: JobContext<DummyJob> = JobContext::new(providers);
    let DummyJobFailure { channel_state, failures: _ } = wait_until_job_failed(job_context.schedule(order1));
    assert_eq!(channel_state.is_reset, true);
}
