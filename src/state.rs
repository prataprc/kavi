use crossterm::{
    self, cursor as term_cursor, event as ct_event,
    event::{DisableMouseCapture, EnableMouseCapture, Event as TermEvent},
    execute, queue,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use log::trace;

use std::{
    io::{self, Write},
    time::{Duration, SystemTime},
};

use crate::{
    buffer::Buffer,
    config::Config,
    event::Event,
    window::{Cursor, Window},
    Error, Result,
};

// Application state
pub struct State {
    pub tm: Terminal,
    pub config: Config,
    pub buffers: Vec<Buffer>,
}

impl AsRef<Config> for State {
    fn as_ref(&self) -> &Config {
        &self.config
    }
}

impl State {
    pub fn new(config: Config) -> Result<State> {
        let tm = Terminal::init()?;
        Ok(State {
            tm,
            config,
            buffers: Default::default(),
        })
    }

    pub fn event_loop(mut self, mut w: Window, mut evnt: Event) -> Result<()> {
        let mut stdout = io::stdout();
        let mut stats = Latency::new();

        let mut start = SystemTime::now();

        // TODO: later statistics can be moved to a different release stream
        // and or controlled by command line option.
        let res = loop {
            // hide cursor, handle event and refresh window
            let _evnt = match evnt {
                Event::Noop => Event::Noop,
                evnt => {
                    err_at!(Fatal, queue!(stdout, term_cursor::Hide))?;
                    let evnt = w.on_event(&mut self, evnt)?;
                    w.on_refresh(&mut self)?;
                    evnt
                }
            };
            // show-cursor
            let Cursor { col, row } = w.to_cursor();
            err_at!(Fatal, queue!(stdout, term_cursor::MoveTo(col, row)))?;
            err_at!(Fatal, queue!(stdout, term_cursor::Show))?;
            err_at!(Fatal, stdout.flush())?;

            stats.sample(start.elapsed().unwrap());
            // new event
            evnt = {
                let tevnt: TermEvent = err_at!(Fatal, ct_event::read())?;
                trace!("{:?} Cursor:({},{})", tevnt, col, row);
                match tevnt.clone().into() {
                    Event::Char('q', m) if m.is_empty() => break Ok(()),
                    evnt => evnt,
                }
            };
            start = SystemTime::now();
        };

        stats.pretty_print("");

        res
    }

    pub fn take_buffer(&mut self, id: &str) -> Option<Buffer> {
        let i = {
            let mut iter = self.buffers.iter().enumerate();
            loop {
                match iter.next() {
                    Some((i, b)) if b.to_id() == id => break Some(i),
                    None => break None,
                    _ => (),
                }
            }
        };
        match i {
            Some(i) => Some(self.buffers.remove(i)),
            None => None,
        }
    }

    pub fn add_buffer(&mut self, buffer: Buffer) {
        self.buffers.insert(0, buffer)
    }
}

impl State {
    pub fn as_buffer(&self, id: &str) -> &Buffer {
        for b in self.buffers.iter() {
            if b.to_id() == id {
                return b;
            }
        }
        unreachable!()
    }

    pub fn as_mut_buffer(&mut self, id: &str) -> &mut Buffer {
        for b in self.buffers.iter_mut() {
            if b.to_id() == id {
                return b;
            }
        }
        unreachable!()
    }

    pub fn to_buffer_num(&self, id: String) -> Option<usize> {
        for b in self.buffers.iter() {
            if b.to_id() == id {
                return Some(b.to_num());
            }
        }
        None
    }
}

pub struct Terminal {
    stdout: io::Stdout,
    pub cols: u16,
    pub rows: u16,
}

impl Terminal {
    fn init() -> Result<Terminal> {
        let mut stdout = io::stdout();
        err_at!(Fatal, terminal::enable_raw_mode())?;
        err_at!(
            Fatal,
            execute!(
                stdout,
                EnterAlternateScreen,
                EnableMouseCapture,
                term_cursor::Hide
            )
        )?;

        let (cols, rows) = err_at!(Fatal, terminal::size())?;
        Ok(Terminal { stdout, cols, rows })
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        execute!(
            self.stdout,
            LeaveAlternateScreen,
            DisableMouseCapture,
            term_cursor::Show
        )
        .unwrap();
        terminal::disable_raw_mode().unwrap();
    }
}

#[derive(Clone, Default, Debug)]
struct Latency {
    samples: usize,
    min: Duration,
    max: Duration,
    total: Duration,
    durations: Vec<usize>,
}

impl Latency {
    fn new() -> Latency {
        let mut stats: Latency = Default::default();
        stats.durations = Vec::with_capacity(256);
        stats.durations.resize(256, 0);
        stats
    }

    fn sample(&mut self, duration: Duration) {
        self.samples += 1;
        self.total += duration;
        if self.min == Duration::from_nanos(0) || self.min > duration {
            self.min = duration
        }
        if self.max == Duration::from_nanos(0) || self.max < duration {
            self.max = duration
        }
        let off: usize = (duration.as_nanos() / 10_000_000) as usize;
        self.durations[off] += 1;
    }

    #[allow(dead_code)]
    fn samples(&self) -> usize {
        self.samples
    }

    #[allow(dead_code)]
    fn min(&self) -> Duration {
        self.min
    }

    #[allow(dead_code)]
    fn max(&self) -> Duration {
        self.max
    }

    fn mean(&self) -> Duration {
        self.total / (self.samples as u32)
    }

    fn percentiles(&self) -> Vec<(u8, usize)> {
        let mut percentiles: Vec<(u8, usize)> = vec![];
        let (mut acc, mut prev_perc) = (0_f64, 90_u8);
        let iter = self
            .durations
            .iter()
            .enumerate()
            .filter(|(_, &item)| item > 0);
        for (duration, samples) in iter {
            acc += *samples as f64;
            let perc = ((acc / (self.samples as f64)) * 100_f64) as u8;
            if perc >= prev_perc {
                percentiles.push((perc, duration));
                prev_perc = perc;
            }
        }
        percentiles
    }

    fn pretty_print(&self, prefix: &str) {
        let mean = self.mean();
        println!(
            "{}duration (min, avg, max): {:?}",
            prefix,
            (self.min, mean, self.max)
        );
        for (duration, n) in self.percentiles().into_iter() {
            if n > 0 {
                println!("{}  {} percentile = {}", prefix, duration, n);
            }
        }
    }
}