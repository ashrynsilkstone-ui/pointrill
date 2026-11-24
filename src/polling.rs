use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, SyncSender, TryRecvError},
};
use std::thread;
use std::time::{Duration, Instant};

use crate::diagnostics::{EVENT_SIZE, device_inputs, input_events};
use crate::model::Device;

const INTERVAL: Duration = Duration::from_millis(250);
pub const HISTORY: f64 = 15.0;

pub struct Sample {
    pub elapsed: f64,
    pub hz: Option<f64>,
    pub cpu: Option<f64>,
}

pub struct PollingTest {
    worker: Option<Sampler>,
    pub samples: VecDeque<Sample>,
    pub message: Option<&'static str>,
}

struct Sampler {
    stop: Arc<AtomicBool>,
    updates: Receiver<Sample>,
}

impl Drop for Sampler {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl PollingTest {
    pub fn start(device: &Device, repaint: impl Fn() + Send + 'static) -> anyhow::Result<Self> {
        let mut files = Vec::new();
        let mut open_error = None;
        for node in device_inputs(device).filter(|node| node.has_relative_motion()) {
            match node.open() {
                Ok(file) => files.push((file, MotionReports::default())),
                Err(error) => open_error = Some(error),
            }
        }
        if files.is_empty() {
            return Err(match open_error {
                Some(error) if error.kind() == io::ErrorKind::PermissionDenied => anyhow::anyhow!(
                    "Kein Lesezugriff auf die Maus-Eingabeschnittstelle. Prüfe die Geräteberechtigungen deiner Sitzung."
                ),
                Some(error) => anyhow::anyhow!(
                    "Maus-Eingabeschnittstelle konnte nicht geöffnet werden: {error}"
                ),
                None => anyhow::anyhow!(
                    "Für das ausgewählte Gerät wurde keine passende Linux-Bewegungsschnittstelle gefunden."
                ),
            });
        }
        let (updates, receiver) = mpsc::sync_channel(64);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        thread::Builder::new()
            .name("pointrill-polling".into())
            .spawn(move || {
                sample(files, &worker_stop, &updates, &repaint);
                drop(updates);
                repaint();
            })?;
        Ok(Self {
            worker: Some(Sampler {
                stop,
                updates: receiver,
            }),
            samples: VecDeque::new(),
            message: None,
        })
    }

    pub fn running(&self) -> bool {
        self.worker.is_some()
    }

    pub fn stop(&mut self) {
        self.refresh();
        self.worker = None;
    }

    pub fn refresh(&mut self) {
        let Some(worker) = &self.worker else {
            return;
        };
        loop {
            match worker.updates.try_recv() {
                Ok(sample) => {
                    let cutoff = sample.elapsed - HISTORY;
                    self.samples.push_back(sample);
                    while self
                        .samples
                        .front()
                        .is_some_and(|sample| sample.elapsed < cutoff)
                    {
                        self.samples.pop_front();
                    }
                }
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    self.worker = None;
                    self.message =
                        Some("Maus-Eingabeschnittstelle getrennt oder Messung abgebrochen.");
                    return;
                }
            }
        }
    }
}

#[derive(Default)]
struct MotionReports {
    reports: u32,
    moved: bool,
    resync: bool,
    incomplete: bool,
}

impl MotionReports {
    fn event(&mut self, kind: u16, code: u16, value: i32) {
        match (kind, code) {
            (0, 3) => {
                // SYN_DROPPED marks a buffer overrun. Discard through the next report.
                self.moved = false;
                self.resync = true;
                self.incomplete = true;
            }
            (0, 0) => {
                if self.moved && !self.resync {
                    self.reports += 1;
                }
                self.moved = false;
                self.resync = false;
            }
            (2, 0 | 1) if value != 0 && !self.resync => self.moved = true,
            _ => {}
        }
    }

    fn take(&mut self) -> Option<u32> {
        let reports = std::mem::take(&mut self.reports);
        let incomplete = std::mem::take(&mut self.incomplete);
        (!incomplete && !self.resync).then_some(reports)
    }
}

fn read_reports(file: &mut File, counts: &mut MotionReports, buffer: &mut [u8]) -> bool {
    for _ in 0..16 {
        match file.read(buffer) {
            Ok(0) => return false,
            Ok(length) => {
                for (kind, code, value) in input_events(&buffer[..length]) {
                    counts.event(kind, code, value);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return false,
        }
    }
    true
}

fn sample(
    mut files: Vec<(File, MotionReports)>,
    stop: &AtomicBool,
    updates: &SyncSender<Sample>,
    repaint: &impl Fn(),
) {
    let mut poll_fds: Vec<_> = files
        .iter()
        .map(|(file, _)| libc::pollfd {
            fd: file.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        })
        .collect();
    let mut buffer = [0; EVENT_SIZE * 256];
    let start = Instant::now();
    let mut interval = start;
    let mut cpu = read_cpu();
    let mut disconnected = false;
    while !stop.load(Ordering::Relaxed) {
        let remaining = INTERVAL.saturating_sub(interval.elapsed());
        let timeout = remaining.as_millis().clamp(1, 20) as libc::c_int;
        // SAFETY: poll_fds is a live and exclusively borrowed array of pollfd records.
        let ready = unsafe {
            libc::poll(
                poll_fds.as_mut_ptr(),
                poll_fds.len() as libc::nfds_t,
                timeout,
            )
        };
        if ready < 0 {
            if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            break;
        }
        for (fd, (file, counts)) in poll_fds.iter_mut().zip(&mut files) {
            if fd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0
                || (fd.revents & libc::POLLIN != 0 && !read_reports(file, counts, &mut buffer))
            {
                disconnected = true;
                fd.fd = -1;
            }
        }
        if poll_fds.iter().all(|fd| fd.fd < 0) {
            break;
        }

        let now = Instant::now();
        let duration = now - interval;
        if duration >= INTERVAL {
            // Some receivers expose duplicate motion interfaces. The busiest
            // interface represents the mouse without summing duplicate reports.
            let mut reports = 0;
            let mut complete = !std::mem::take(&mut disconnected);
            for (fd, (_, counts)) in poll_fds.iter().zip(&mut files) {
                if fd.fd >= 0 {
                    if let Some(count) = counts.take() {
                        reports = reports.max(count);
                    } else {
                        complete = false;
                    }
                }
            }
            let next_cpu = read_cpu();
            let load = cpu
                .zip(next_cpu)
                .and_then(|(prev, next)| cpu_load(prev, next));
            cpu = next_cpu;
            let sample = Sample {
                elapsed: (now - start).as_secs_f64(),
                hz: complete.then(|| f64::from(reports) / duration.as_secs_f64()),
                cpu: load,
            };
            if updates.try_send(sample).is_ok() {
                repaint();
            }
            interval = now;
        }
    }
}

fn read_cpu() -> Option<[u64; 8]> {
    cpu_ticks(&fs::read_to_string("/proc/stat").ok()?)
}

fn cpu_ticks(stat: &str) -> Option<[u64; 8]> {
    let mut fields = stat.lines().next()?.split_whitespace();
    if fields.next()? != "cpu" {
        return None;
    }
    let mut ticks = [0; 8];
    for tick in &mut ticks {
        *tick = fields.next()?.parse().ok()?;
    }
    Some(ticks)
}

fn cpu_load(before: [u64; 8], after: [u64; 8]) -> Option<f64> {
    let mut elapsed = 0;
    let mut active = 0;
    for (index, (before, after)) in before.into_iter().zip(after).enumerate() {
        // iowait may decrease. Discard other counter resets.
        let delta = if index == 4 {
            after.saturating_sub(before)
        } else {
            after.checked_sub(before)?
        };
        elapsed += delta;
        if !matches!(index, 3 | 4) {
            active += delta;
        }
    }
    (elapsed > 0).then(|| 100.0 * active as f64 / elapsed as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_movement_packets_not_axes_and_discards_overruns() {
        let mut reports = MotionReports::default();
        for event in [(2, 0, 1), (2, 1, -3), (0, 0, 0), (2, 8, 1), (0, 0, 0)] {
            reports.event(event.0, event.1, event.2);
        }
        assert_eq!(reports.take(), Some(1));
        reports.event(0, 3, 0);
        reports.event(2, 0, 5);
        assert_eq!(reports.take(), None);
        reports.event(0, 0, 0);
        assert_eq!(reports.take(), Some(0));
        reports.event(2, 0, 0);
        reports.event(0, 0, 0);
        assert_eq!(reports.take(), Some(0));
    }

    #[test]
    fn cpu_excludes_idle_iowait_and_duplicate_guest_time() {
        let before = cpu_ticks("cpu 100 0 30 500 10 0 0 0 20 0\ncpu0 1").unwrap();
        let after = cpu_ticks("cpu 110 0 40 568 20 0 2 0 28 0").unwrap();
        assert_eq!(cpu_load(before, after), Some(22.0));
        assert_eq!(cpu_load(after, before), None);
        assert_eq!(cpu_load(after, after), None);
    }
}
