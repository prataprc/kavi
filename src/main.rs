#![feature(backtrace)]
#![feature(box_syntax)]

use crossterm::{
    self, cursor,
    event::{self as ct_event, Event as TermEvent},
    event::{DisableMouseCapture, EnableMouseCapture},
    execute, queue,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use dirs;
use log::trace;
use simplelog;
use structopt::StructOpt;

use std::{
    fs,
    io::{self, Write},
    path,
};

use ted::{
    err_at,
    window::{Context, Coord, Cursor},
    window_file::WindowFile,
    Buffer, Config, Error, Event, Result, Window,
};

#[derive(Debug, StructOpt)]
pub struct Opt {
    #[structopt(long = "log", default_value = "")]
    log_file: String,

    #[structopt(short = "v", long = "verbose")]
    verbose: bool,

    #[structopt(long = "trace")]
    trace: bool,

    files: Vec<String>,
}

fn main() {
    let opts = Opt::from_args();

    match init_logger(&opts) {
        Ok(()) => (),
        Err(err) => {
            println!("{}", err);
            std::process::exit(1);
        }
    }

    std::panic::set_hook(box |panic_info| {
        let mut s = format!(
            "panic occured: {:?}",
            panic_info.payload().downcast_ref::<String>().unwrap()
        );
        s.push_str(&format!("{}", std::backtrace::Backtrace::capture()));
        fs::write("ted-panic.out", s.as_bytes()).unwrap();
    });

    match Application::run(opts) {
        Ok(_) => (),
        Err(err) => println!("{}", err),
    }
}

struct Application {
    tm: Terminal,
    window: Option<Box<dyn Window>>,
    context: Context,
}

impl Application {
    pub fn run(_opts: Opt) -> Result<()> {
        let config: Config = Default::default();
        let mut app = {
            let tm = Terminal::init()?;
            let coord = Coord::new(1, 1, tm.rows, tm.cols);
            let w = err_at!(Fatal, WindowFile::new(coord, config.clone()))?;
            Application {
                tm,
                window: Some(Box::new(w)),
                context: Context::new(config.clone()),
            }
        };

        // TODO: for now assume that file has r/w permission
        //for file in opts.files.iter() {
        //    let file_loc = util::to_file_loc(file.as_ref())?;
        //    let f = {
        //        let mut opts = fs::OpenOptions::new();
        //        err_at!(Fatal, opts.read(true).write(true).open(&file_loc))?
        //    };
        //    let mut buffer = Buffer::from_reader(f, config.clone())?;
        //    buffer.set_file_loc(&file_loc);
        //    app.buffers.push(buffer);
        //}

        let (buffer_id, buffer) = {
            let mut b = Buffer::empty(config)?;
            b.set_location(Default::default());
            (b.to_id(), b)
        };
        app.context.buffers.push(buffer);
        match app.window.take() {
            Some(mut window) => {
                window.handle_event(&mut app.context, Event::EditBuffer { buffer_id });
                app.window = Some(window);
            }
            None => err_at!(Fatal, msg: format!("unreachable"))?,
        };

        app.event_loop()
    }

    fn event_loop(mut self) -> Result<()> {
        loop {
            let (evnt, _tevnt): (Event, TermEvent) = {
                let tevnt: TermEvent = err_at!(Fatal, ct_event::read())?;
                trace!("Event-{:?}", tevnt);
                (tevnt.clone().into(), tevnt)
            };

            err_at!(Fatal, queue!(self.tm.stdout, cursor::Hide))?;

            let cursor = match self.window.take() {
                Some(mut window) => {
                    match window.handle_event(&mut self.context, evnt)? {
                        Some(Event::Char('q', m)) if m.is_empty() => {
                            //
                            break Ok(());
                        }
                        _ => (),
                    }
                    err_at!(Fatal, window.refresh(&mut self.context))?;
                    let cursor = window.to_cursor();
                    self.window = Some(window);
                    Some(cursor)
                }
                None => {
                    err_at!(Fatal, msg: format!("unreachable"))?;
                    None
                }
            };

            match cursor {
                Some(Cursor { col, row }) => {
                    err_at!(Fatal, queue!(self.tm.stdout, cursor::MoveTo(col, row)))?;
                    err_at!(Fatal, queue!(self.tm.stdout, cursor::Show))?;
                    err_at!(Fatal, self.tm.stdout.flush())?;
                }
                None => (),
            }
        }
    }
}

struct Terminal {
    stdout: io::Stdout,
    cols: u16,
    rows: u16,
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
                cursor::Hide
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
            cursor::Show
        )
        .unwrap();
        terminal::disable_raw_mode().unwrap();
    }
}

fn init_logger(opts: &Opt) -> Result<()> {
    if opts.log_file.is_empty() {
        Ok(())
    } else {
        let log_file: path::PathBuf = [
            err_at!(
                Fatal,
                dirs::home_dir().ok_or(format!("can't find home-directory"))
            )?,
            path::Path::new(&opts.log_file).to_path_buf(),
        ]
        .iter()
        .collect();

        let level_filter = if opts.trace {
            simplelog::LevelFilter::Trace
        } else if opts.verbose {
            simplelog::LevelFilter::Debug
        } else {
            simplelog::LevelFilter::Info
        };

        let mut config = simplelog::ConfigBuilder::new();
        config
            .set_location_level(simplelog::LevelFilter::Error)
            .set_target_level(simplelog::LevelFilter::Off)
            .set_thread_mode(simplelog::ThreadLogMode::Both)
            .set_thread_level(simplelog::LevelFilter::Error)
            .set_time_to_local(true)
            .set_time_format("%Y-%m-%dT%H-%M-%S%.3f".to_string());

        let fs = err_at!(Fatal, fs::File::create(&log_file))?;
        err_at!(
            Fatal,
            simplelog::WriteLogger::init(level_filter, config.build(), fs)
        )?;

        Ok(())
    }
}
