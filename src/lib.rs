use std::{
    io, time::{Duration, Instant, SystemTime}
};

use average::AtomicAverage;
pub use range::Range;

mod average;
mod http;
mod range;

const SECOND: Duration = Duration::from_secs(1);

pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

fn time_diff(server: SystemTime, machine: SystemTime) -> i16 {
    match server.duration_since(machine) {
        Ok(x) => x.as_millis() as i16,
        Err(e) => -(e.duration().as_millis() as i16)
    }
}

pub struct Client {
    client: http::Client,
    range: Range,
    prev_server_time: SystemTime,
    iteration: u32
}

impl Client {
    pub async fn new(url: impl AsRef<str>, port: u16) -> io::Result<Self> {
        let url = url.as_ref();

        let client = http::Client::new(url, port).await?;
        let range = Range::new(i16::MIN, i16::MAX);
        
        Ok( Self {
            client, range,
            prev_server_time: SystemTime::UNIX_EPOCH,
            iteration: 0
        } )
    }

    /// Sends a request to try and reduce the amount of deviation in the possible time space.
    /// 
    /// Returns a Range which contains the minimum and maximum possible time deviation
    /// between the server and client calculated up until this run, in units of milliseconds.
    pub async fn run(&mut self) -> io::Result<Range> {
        /// Returns the calculated difference between machine and server, assuming that the time of generation
        /// for the response is the middlepoint of send and recv.
        fn get_diff(server: SystemTime, machine: SystemTime) -> Range {
            let diff = time_diff(server, machine);
            Range {
                min: diff,
                max: diff + 1000
            }
        }

        async fn sleep(base: Instant, dur: Duration) -> bool {
            //Correction value for oversleeping
            static CORRECTION: AtomicAverage = AtomicAverage::new();
    
            let mut until = base + dur - CORRECTION.get();
            let now = Instant::now();
    
            let (dur, adjusted) = match until.checked_duration_since(now) {
                Some(x) => (x, false), // until > now
                None => {
                    until += SECOND;
                    (until - now, true)
                }
            };
    
            tokio::time::sleep(dur).await;
            CORRECTION.add(until.elapsed());
            adjusted
        }

        let ping = self.client.get_date().await?;
        let range = get_diff(ping.server_time, ping.machine_time);
        self.range.transpose(range);

        let dur = if self.prev_server_time == ping.server_time {
            SECOND / 2u32.pow(self.iteration + 1)
        } else {
            SECOND - SECOND / 2u32.pow(self.iteration + 1)
        };

        let adjusted = sleep(ping.initiated_at, dur).await;
        
        self.prev_server_time = ping.server_time + SECOND * adjusted as u32;
        self.iteration += 1;

        Ok(self.range)
    }

    pub fn range(&self) -> Range {
        self.range
    }
}