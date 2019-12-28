#![feature(vec_remove_item)]
#![feature(with_options)]

extern crate http;
extern crate rand;
extern crate flexo;

use std::{io, str};
use curl::easy::{Easy2, Handler, WriteError};
use std::io::prelude::*;
use http::Uri;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::BufWriter;
use flexo::*;

static DIRECTORY: &str = "/tmp/curl_ex_out/";

#[derive(PartialEq, Eq, Hash, Clone, Debug)]
struct DownloadProvider {
    uri: Uri,
}

impl Provider <DownloadJob, DownloadOrder, DownloadChannel, DownloadJobError, Uri, i32, FileState, DownloadChannelState> for DownloadProvider {
    fn new_job(&self, order: DownloadOrder) -> DownloadJob {
        let uri_string = format!("{}/{}", self.uri, order.filepath);
        let uri = uri_string.parse::<Uri>().unwrap();
        let provider = self.clone();
        DownloadJob {
            provider,
            uri,
            order
        }
    }

    fn identifier(&self) -> &Uri {
        &self.uri
    }

    fn score(&self) -> i32 {
        // TODO
        0
    }
}

#[derive(Debug)]
pub enum DownloadJobError {
    CurlError(curl::Error),
    HttpFailureStatus(u32),
}

#[derive(Debug)]
struct DownloadJob {
    provider: DownloadProvider,
    uri: Uri,
    order: DownloadOrder,
}

impl Job <DownloadChannel, DownloadOrder, DownloadProvider, DownloadJobError, Uri, i32, FileState, DownloadChannelState> for DownloadJob {
    fn provider(&self) -> &DownloadProvider {
        &self.provider
    }

    fn order(&self) -> &DownloadOrder {
        &self.order
    }

    fn execute(self, mut channel: DownloadChannel) -> JobResult<DownloadChannel, DownloadProvider, DownloadJobError> {
        let url = format!("{}", &self.uri);
        channel.handle.url(&url).unwrap();
        // Limit the speed to facilitate debugging.
        channel.handle.max_recv_speed(3495253 * 2).unwrap();
        channel.handle.low_speed_time(std::time::Duration::from_secs(8)).unwrap();
        channel.handle.low_speed_limit(524288000).unwrap();
        channel.handle.follow_location(true).unwrap();
        match channel.progress_indicator() {
            None => {},
            Some(start) => {
                channel.handle.resume_from(start).unwrap();
            }
        }
        let result = match channel.handle.perform() {
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
        };
        result
    }
}

#[derive(PartialEq, Eq, Hash, Clone, Debug)]
struct DownloadOrder {
    /// This path is relative to the given root directory.
    filepath: String,
}

impl Order<DownloadChannel> for DownloadOrder {
    fn new_channel(self) -> DownloadChannel {
        DownloadChannel {
            handle: Easy2::new(DownloadState::new(self).unwrap()),
            state: DownloadChannelState::new(),
        }
    }
}

#[derive(PartialEq, Eq, Hash, Clone, Debug, Copy)]
struct DownloadChannelState {
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
    fn reset(&mut self) {
        self.is_reset = true;
    }
}

#[derive(Debug)]
struct FileState {
    buf_writer: BufWriter<File>,
    size_written: u64,
}

impl JobState for FileState {}

impl DownloadState {
    pub fn new(order: DownloadOrder) -> std::io::Result<Self> {
        let f = OpenOptions::new().create(true).append(true).open(DIRECTORY.to_owned() + &order.filepath)?;
        let size_written = f.metadata()?.len();
        let buf_writer = BufWriter::new(f);
        let job_state = JobStateItem {
            order,
            state: Some(FileState {
                buf_writer,
                size_written,
            }),
        };
        Ok(DownloadState { job_state })
    }

    pub fn reset(&mut self, order: DownloadOrder) -> std::io::Result<()> {
        if order != self.job_state.order {
            let c = DownloadState::new(order.clone())?;
            self.job_state.state = c.job_state.state;
            self.job_state.order = order;
        }
        Ok(())
    }
}

#[derive(Debug)]
struct DownloadState {
    job_state: JobStateItem<DownloadOrder, FileState>
}

impl Handler for DownloadState {
    fn write(&mut self, data: &[u8]) -> Result<usize, WriteError> {
        match self.job_state.state.iter_mut().next() {
            None => panic!("Expected the state to be initialized."),
            Some(file_state) => {
                file_state.size_written += data.len() as u64;
                match file_state.buf_writer.write(data) {
                    Ok(size) => {
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
struct DownloadChannel {
    handle: Easy2<DownloadState>,
    state: DownloadChannelState,
}

impl Channel <DownloadOrder, FileState, DownloadChannelState> for DownloadChannel {
    fn progress_indicator(&self) -> Option<u64> {
        let file_state = self.handle.get_ref().job_state.state.as_ref().unwrap();
        let size_written = file_state.size_written;
        if size_written > 0 {
            Some(size_written)
        } else {
            None
        }
    }

    fn reset_order(&mut self, order: DownloadOrder) {
        self.handle.get_mut().reset(order).unwrap();
    }

    fn channel_state_item(&mut self) -> &mut JobStateItem<DownloadOrder, FileState> {
        &mut self.handle.get_mut().job_state
    }

    fn channel_state(&self) -> DownloadChannelState {
        self.state
    }

    fn channel_state_ref(&mut self) -> &mut DownloadChannelState {
        &mut self.state
    }
}

fn initial_providers() -> Vec<DownloadProvider> {
    let mut providers = Vec::new();
    providers.push(DownloadProvider { uri: "https://mirror.yandex.ru/archlinux/".parse::<Uri>().unwrap() });
    providers.push(DownloadProvider { uri: "https://mirror.yandex.ru/archlinux/".parse::<Uri>().unwrap() });
    providers.push(DownloadProvider { uri: "https://mirror.yandex.ru/archlinux/".parse::<Uri>().unwrap() });
    providers.push(DownloadProvider { uri: "https://mirror.yandex.ru/archlinux/".parse::<Uri>().unwrap() });
    providers.push(DownloadProvider { uri: "https://mirror.yandex.ru/archlinux/".parse::<Uri>().unwrap() });
    providers.push(DownloadProvider { uri: "https://mirror.yandex.ru/archlinux/".parse::<Uri>().unwrap() });
    providers.push(DownloadProvider { uri: "https://mirror.yandex.ru/archlinux/".parse::<Uri>().unwrap() });
    providers.push(DownloadProvider { uri: "https://mirror.yandex.ru/archlinux/".parse::<Uri>().unwrap() });
    providers.push(DownloadProvider { uri: "https://mirror.yandex.ru/archlinux/".parse::<Uri>().unwrap() });
    providers.push(DownloadProvider { uri: "https://mirror.yandex.ru/archlinux/".parse::<Uri>().unwrap() });
    providers.push(DownloadProvider { uri: "https://mirror.yandex.ru/archlinux/".parse::<Uri>().unwrap() });
    providers.push(DownloadProvider { uri: "https://mirror.yandex.ru/archlinux/".parse::<Uri>().unwrap() });
    providers.push(DownloadProvider { uri: "https://mirror.yandex.ru/archlinux/".parse::<Uri>().unwrap() });

    providers
}

fn main() {
    let mut job_context = JobContext::new(initial_providers());

    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let filename: String = line.unwrap();
        let order = DownloadOrder {
            filepath: String::from(filename),
        };
        job_context.schedule(order);
    }
}
