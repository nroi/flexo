extern crate flexo;

use crate::mirror_config::{MirrorsAutoConfig, MirrorConfig};
use crate::mirror_fetch;
use crate::mirror_fetch::MirrorUrl;

use flexo::*;
use http::Uri;
use std::fs::File;
use std::time::Duration;
use std::cmp::Ordering;
use crossbeam::crossbeam_channel::Sender;
use curl::easy::{Easy2, Handler, WriteError};
use std::fs::OpenOptions;
use std::io::BufWriter;
use std::io::prelude::*;

pub static DIRECTORY: &str = "/tmp/curl_ex_out/";

#[derive(PartialEq, Eq, Hash, Clone, Debug)]
pub struct DownloadProvider {
    pub uri: Uri,
    pub mirror_results: MirrorResults,
    pub country: String,
}

impl Provider for DownloadProvider {
    type J = DownloadJob;

    fn new_job(&self, order: DownloadOrder) -> DownloadJob {
        let uri_string = format!("{}/{}", self.uri, order.filepath);
        let uri = uri_string.parse::<Uri>().unwrap();
        let provider = self.clone();
        DownloadJob {
            provider,
            uri,
            order,
        }
    }

    fn identifier(&self) -> &Uri {
        &self.uri
    }

    fn score(&self) -> MirrorResults {
        self.mirror_results
    }
}

#[derive(PartialEq, Eq, Hash, Copy, Clone, Debug, Default)]
pub struct MirrorResults {
    pub namelookup_duration: Duration,
    pub connect_duration: Duration,
}

impl Ord for MirrorResults {
    fn cmp(&self, other: &Self) -> Ordering {
        self.connect_duration.cmp(&other.connect_duration)
    }
}

impl PartialOrd for MirrorResults {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug)]
pub enum DownloadJobError {
    CurlError(curl::Error),
    HttpFailureStatus(u32),
}

#[derive(Debug)]
pub struct DownloadJob {
    provider: DownloadProvider,
    uri: Uri,
    order: DownloadOrder,
}

impl Job for DownloadJob {
    type S = MirrorResults;
    type JS = FileState;
    type C = DownloadChannel;
    type O = DownloadOrder;
    type P = DownloadProvider;
    type E = DownloadJobError;
    type CS = DownloadChannelState;
    type PI = Uri;
    type PR = MirrorsAutoConfig;

    fn provider(&self) -> &DownloadProvider {
        &self.provider
    }

    fn order(&self) -> DownloadOrder {
        self.order.clone()
    }

    fn execute(self, mut channel: DownloadChannel, properties: MirrorsAutoConfig) -> JobResult<DownloadJob> {
        let url = format!("{}", &self.uri);
        channel.handle.url(&url).unwrap();
        // Limit the speed to facilitate debugging.
        // TODO disable the speed limit before releasing this.
        match properties.low_speed_limit {
            None => {},
            Some(speed) => {
                channel.handle.low_speed_limit(speed).unwrap();
                let low_speed_time_secs = properties.low_speed_time_secs.unwrap_or(4);
                channel.handle.low_speed_time(std::time::Duration::from_secs(low_speed_time_secs)).unwrap();
            },
        }
        match properties.max_speed_limit {
            None => {},
            Some(speed) => {
                channel.handle.max_recv_speed(speed).unwrap();
            },
        }
        channel.handle.follow_location(true).unwrap();
        channel.handle.max_redirections(3).unwrap();
        match channel.progress_indicator() {
            None => {},
            Some(start) => {
                channel.handle.resume_from(start).unwrap();
            }
        }
        match channel.handle.perform() {
            Ok(()) => {
                let response_code = channel.handle.response_code().unwrap();
                if response_code >= 200 && response_code < 300 {
                    println!("Success!");
                    JobResult::Complete(JobCompleted::new(channel, self.provider))
                } else {
                    let termination = JobTerminated {
                        channel,
                        error: DownloadJobError::HttpFailureStatus(response_code),
                    };
                    JobResult::Error(termination)
                }
            },
            Err(e) => {
                match channel.progress_indicator() {
                    Some(size) if size > 0 => {
                        JobResult::Partial(JobPartiallyCompleted::new(channel, size))
                    }
                    _ => {
                        let termination = JobTerminated {
                            channel,
                            error: DownloadJobError::CurlError(e),
                        };
                        JobResult::Error(termination)
                    }
                }
            }
        }
    }
}

#[derive(PartialEq, Eq, Hash, Clone, Debug)]
pub struct DownloadOrder {
    /// This path is relative to the given root directory.
    pub filepath: String,
}

impl Order for DownloadOrder {
    type J = DownloadJob;

    fn new_channel(self, tx: Sender<FlexoProgress>) -> DownloadChannel {
        DownloadChannel {
            handle: Easy2::new(DownloadState::new(self, tx).unwrap()),
            state: DownloadChannelState::new(),
        }
    }
}

#[derive(PartialEq, Eq, Hash, Clone, Debug, Copy)]
pub struct DownloadChannelState {
    is_reset: bool,
}

impl DownloadChannelState {
    fn new() -> Self {
        Self {
            is_reset: false
        }
    }
}

impl ChannelState for DownloadChannelState {
    type J = DownloadJob;

    fn reset(&mut self) {
        self.is_reset = true;
    }
}

#[derive(Debug)]
pub struct FileState {
    buf_writer: BufWriter<File>,
    size_written: u64,
}

impl JobState for FileState {
    type J = DownloadJob;
}

#[derive(Debug)]
struct DownloadState {
    job_state: JobStateItem<DownloadJob>
}

impl DownloadState {
    pub fn new(order: DownloadOrder, tx: Sender<FlexoProgress>) -> std::io::Result<Self> {
        let path = DIRECTORY.to_owned() + &order.filepath;
        println!("Attempt to create file: {:?}", path);
        let f = OpenOptions::new().create(true).append(true).open(path)?;
        let size_written = f.metadata()?.len();
        let buf_writer = BufWriter::new(f);
        let job_state = JobStateItem {
            order,
            state: Some(FileState {
                buf_writer,
                size_written,
            }),
            tx,
        };
        Ok(DownloadState { job_state })
    }

    pub fn reset(&mut self, order: DownloadOrder, tx: Sender<FlexoProgress>) -> std::io::Result<()> {
        if order != self.job_state.order {
            let c = DownloadState::new(order.clone(), tx)?;
            self.job_state.state = c.job_state.state;
            self.job_state.order = order;
        }
        Ok(())
    }
}

impl Handler for DownloadState {
    fn write(&mut self, data: &[u8]) -> Result<usize, WriteError> {
        match self.job_state.state.iter_mut().next() {
            None => panic!("Expected the state to be initialized."),
            Some(file_state) => {
                file_state.size_written += data.len() as u64;
                match file_state.buf_writer.write(data) {
                    Ok(size) => {
                        let len = file_state.buf_writer.get_ref().metadata().unwrap().len();
                        let _result = self.job_state.tx.send(FlexoProgress::Progress(len));
                        Ok(size)
                    },
                    Err(e) => {
                        println!("Error while writing data: {:?}", e);
                        Err(WriteError::Pause)
                    }
                }
            }
        }
    }
}

#[derive(Debug)]
pub struct DownloadChannel {
    handle: Easy2<DownloadState>,
    state: DownloadChannelState,
}

impl Channel for DownloadChannel {
    type J = DownloadJob;

    fn progress_indicator(&self) -> Option<u64> {
        let file_state = self.handle.get_ref().job_state.state.as_ref().unwrap();
        let size_written = file_state.size_written;
        if size_written > 0 {
            Some(size_written)
        } else {
            None
        }
    }

    fn reset_order(&mut self, order: DownloadOrder, tx: Sender<FlexoProgress>) {
        self.handle.get_mut().reset(order, tx).unwrap();
    }

    fn channel_state_item(&mut self) -> &mut JobStateItem<DownloadJob> {
        &mut self.handle.get_mut().job_state
    }

    fn channel_state(&self) -> DownloadChannelState {
        self.state
    }

    fn channel_state_ref(&mut self) -> &mut DownloadChannelState {
        &mut self.state
    }
}

pub fn rate_providers(mut mirror_urls: Vec<MirrorUrl>, mirror_config: &MirrorConfig) -> Vec<DownloadProvider> {
    mirror_urls.sort_by(|a, b| a.score.partial_cmp(&b.score).unwrap());
    let filtered_mirror_urls: Vec<MirrorUrl> = mirror_urls
        .into_iter()
        .filter(|x| x.filter_predicate(&mirror_config))
        .take(mirror_config.mirrors_auto.num_mirrors)
        .collect();
    let mut mirrors_with_latencies = Vec::new();
    let timeout = Duration::from_millis(mirror_config.mirrors_auto.timeout);
    for mirror in filtered_mirror_urls.into_iter() {
        match mirror_fetch::measure_latency(&mirror.url, timeout) {
            None => {},
            Some(latency) => {
                mirrors_with_latencies.push((mirror, latency));
            }
        }
    }
    mirrors_with_latencies.sort_unstable_by_key(|(_, latency)| {
        *latency
    });

    mirrors_with_latencies.into_iter().map(|(mirror, mirror_results)| {
        DownloadProvider {
            uri: mirror.url.parse::<Uri>().unwrap(),
            mirror_results,
            country: mirror.country,
        }
    }).collect()
}

