mod cmd;
mod cmd_set;
//mod cmd_edit;
//mod cmd_file;
//mod cmd_write;

mod col_nu;
mod config;
mod ftype;
mod ftype_txt_en;
mod keymap;
mod keymap_edit;
mod view;
mod window_edit;
mod window_file;
mod window_line;
mod window_prompt;

use crossterm::{cursor as term_cursor, queue};
use log::trace;

use std::{
    ffi,
    io::{self, Write},
    mem,
    sync::mpsc,
};

use crate::{
    buffer::Buffer,
    code::cmd::Command,
    code::config::Config,
    code::window_prompt::WindowPrompt,
    code::{window_file::WindowFile, window_line::WindowLine},
    color_scheme::{ColorScheme, Highlight},
    event::Event,
    location::Location,
    pubsub::PubSub,
    state::Opt,
    window::{Coord, Cursor, Notify, Span, Spanline},
    Error, Result,
};

pub struct App {
    coord: Coord,
    config: Config,
    color_scheme: ColorScheme,
    subscribers: PubSub,
    buffers: Vec<Buffer>,

    wfile: WindowFile,
    tbcline: WindowLine,
    inner: Inner,
}

enum Inner {
    Edit {
        stsline: WindowLine,
    },
    AnyKey {
        stsline: WindowLine,
        prompts: Vec<WindowPrompt>,
    },
    Command {
        cmdline: WindowLine,
    },
    None,
}

impl Default for Inner {
    fn default() -> Inner {
        Inner::None
    }
}

impl AsRef<Config> for App {
    fn as_ref(&self) -> &Config {
        &self.config
    }
}

impl AsMut<Config> for App {
    fn as_mut(&mut self) -> &mut Config {
        &mut self.config
    }
}

impl App {
    pub fn new(config: toml::Value, coord: Coord, opts: Opt) -> Result<App> {
        let config = {
            let cnf: Config = Default::default();
            cnf.mixin(config.try_into().unwrap())
        };

        trace!("starting app `code` coord:{} config...\n{}", coord, config);

        let mut app = App {
            coord,
            config,
            color_scheme: Default::default(),
            subscribers: Default::default(),
            buffers: Default::default(),

            wfile: Default::default(),
            tbcline: App::new_tbcline(coord),
            inner: Default::default(),
        };

        let stsline = App::new_stsline(coord);
        let wps = app.open_cmd_files(opts.files.clone())?;
        app.inner = if wps.len() > 0 {
            Inner::AnyKey {
                stsline,
                prompts: wps,
            }
        } else {
            Inner::Edit { stsline }
        };

        App::draw_screen(app.coord, &app.color_scheme)?;

        app.wfile = {
            let wf_coord = {
                let mut wf_coord = coord;
                wf_coord.hgt -= 1;
                wf_coord
            };
            match app.buffers.last() {
                Some(buf) => WindowFile::new(wf_coord, buf, app.as_ref()),
                None => {
                    let buf = Buffer::empty();
                    let wfile = WindowFile::new(wf_coord, &buf, app.as_ref());
                    app.add_buffer(buf);
                    wfile
                }
            }
        };

        Ok(app)
    }

    fn new_stsline(coord: Coord) -> WindowLine {
        let (col, _) = coord.to_origin();
        let (hgt, wth) = coord.to_size();
        WindowLine::new_status(Coord::new(col, hgt, 1, wth))
    }

    fn new_cmdline(coord: Coord) -> WindowLine {
        let (col, _) = coord.to_origin();
        let (hgt, wth) = coord.to_size();
        WindowLine::new_cmd(Coord::new(col, hgt, 1, wth))
    }

    fn new_tbcline(coord: Coord) -> WindowLine {
        let (col, _) = coord.to_origin();
        let (hgt, wth) = coord.to_size();
        let hgt = hgt.saturating_sub(1);
        WindowLine::new_tab(Coord::new(col, hgt, 1, wth))
    }
}

impl App {
    pub fn subscribe(&mut self, topic: &str, tx: mpsc::Sender<Notify>) {
        self.subscribers.subscribe(topic, tx);
    }

    pub fn notify(&self, topic: &str, msg: Notify) -> Result<()> {
        self.subscribers.notify(topic, msg)
    }

    pub fn add_buffer(&mut self, buffer: Buffer) {
        self.buffers.insert(0, buffer)
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
}

impl App {
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

    fn as_color_scheme(&self) -> &ColorScheme {
        &self.color_scheme
    }
}

impl App {
    fn draw_screen(coord: Coord, scheme: &ColorScheme) -> Result<()> {
        use crossterm::style::{SetBackgroundColor, SetForegroundColor};
        use std::iter::{repeat, FromIterator};

        let mut stdout = io::stdout();
        {
            let style = scheme.to_style(Highlight::Canvas);
            err_at!(Fatal, queue!(stdout, SetForegroundColor(style.fg)))?;
            err_at!(Fatal, queue!(stdout, SetBackgroundColor(style.bg)))?;
        }

        let (col, row) = coord.to_origin_cursor();
        let (hgt, wth) = coord.to_size();
        for r in row..(row + hgt) {
            let span: Span = {
                let s = String::from_iter(repeat(' ').take(wth as usize));
                s.into()
            };
            err_at!(Fatal, queue!(stdout, term_cursor::MoveTo(col, r)))?;
            err_at!(Fatal, queue!(stdout, span))?;
        }

        Ok(())
    }

    fn open_cmd_files(&mut self, files: Vec<String>) -> Result<Vec<WindowPrompt>> {
        let locs: Vec<Location> = files
            .into_iter()
            .map(|f| {
                let f: ffi::OsString = f.into();
                Location::new_disk(&f)
            })
            .collect();
        let mut efiles = vec![];
        for loc in locs.into_iter() {
            match loc.to_rw_file() {
                Some(f) => match Buffer::from_reader(f, loc.clone()) {
                    Ok(mut buf) if self.config.read_only => {
                        trace!("opening {} in read-mode", loc);
                        buf.set_read_only(true);
                        self.add_buffer(buf)
                    }
                    Ok(buf) => {
                        trace!("opening {} in write-mode", loc);
                        self.add_buffer(buf)
                    }
                    Err(err) => efiles.push((loc, err)),
                },
                None => match loc.to_r_file() {
                    Some(f) => match Buffer::from_reader(f, loc.clone()) {
                        Ok(mut buf) => {
                            trace!("opening {} in read-mode", loc);
                            buf.set_read_only(true);
                            self.add_buffer(buf);
                        }
                        Err(err) => efiles.push((loc, err)),
                    },
                    None => {
                        let err = "file missing/no-permission".to_string();
                        efiles.push((loc, Error::IOError(err)))
                    }
                },
            }
        }

        let mut wps = vec![];
        let prompt_coord = {
            let (col, row) = self.coord.to_origin();
            let (hgt, wth) = self.coord.to_size();
            Coord::new(col, row, hgt - 1, wth)
        };
        for (loc, err) in efiles.into_iter() {
            let span1 = {
                let st = format!("{:?} : {}", loc.to_long_string()?, err);
                span!(st: st).using(self.color_scheme.to_style(Highlight::Error))
            };
            let span2 = {
                let span = span!(st: format!("-press any key to continue-"));
                span.using(self.color_scheme.to_style(Highlight::Prompt))
            };
            let span_lines: Vec<Spanline> = vec![span1.into(), span2.into()];
            wps.push(WindowPrompt::new(prompt_coord, span_lines));
        }

        Ok(wps)
    }
}

impl App {
    #[inline]
    pub fn post(&mut self, _msg: Notify) -> Result<()> {
        //match msg {
        //    Notify::Status(sl)) -> self.stsline.set(sl),
        //    Notify::TabComplete(sl) -> self.tbcline.set(sl),
        //}
        Ok(())
    }

    pub fn to_cursor(&self) -> Cursor {
        match &self.inner {
            Inner::Edit { .. } => self.wfile.to_cursor(),
            Inner::AnyKey { prompts, .. } => prompts[0].to_cursor(),
            Inner::Command { cmdline } => cmdline.to_cursor(),
            Inner::None => Default::default(),
        }
    }

    pub fn on_event(&mut self, evnt: Event) -> Result<Event> {
        let inner = mem::replace(&mut self.inner, Default::default());
        let (inner, evnt) = inner.on_event(self, evnt)?;
        self.inner = inner;

        match evnt {
            Event::Cmd(name, args) => {
                let mut cmd: Command = (name, args).into();
                cmd.on_command(self)?;
                Ok(Event::Noop)
            }
            evnt => Ok(evnt),
        }
    }

    pub fn on_refresh(&mut self) -> Result<()> {
        let mut wfile = mem::replace(&mut self.wfile, Default::default());
        wfile.on_refresh(self)?;
        self.wfile = wfile;

        let inner = mem::replace(&mut self.inner, Default::default());
        self.inner = inner.on_refresh(self)?;

        //let mut wline = mem::replace(&mut self.tbcline, Default::default());
        //wline.on_refresh(self)?;
        //self.tbcline = wline;

        Ok(())
    }
}

impl Inner {
    fn on_event(self, app: &mut App, evnt: Event) -> Result<(Inner, Event)> {
        match (self, evnt) {
            (Inner::Edit { .. }, Event::Char(':', m)) if m.is_empty() => {
                let mut cmdline = App::new_cmdline(app.coord);
                cmdline.on_event(app, Event::Char(':', m))?;
                Ok((Inner::Command { cmdline }, Event::Noop))
            }
            (Inner::Edit { stsline }, evnt) => {
                let mut wfile = mem::replace(&mut app.wfile, Default::default());
                let evnt = wfile.on_event(app, evnt)?;
                app.wfile = wfile;
                Ok((Inner::Edit { stsline }, evnt))
            }
            (
                Inner::AnyKey {
                    mut prompts,
                    stsline,
                },
                evnt,
            ) => {
                let evnt = prompts[0].on_event(app, evnt)?;
                if prompts[0].prompt_match().is_some() {
                    prompts.remove(0);
                }
                Ok(match prompts.len() {
                    0 => (Inner::AnyKey { prompts, stsline }, evnt),
                    _ => (Inner::Edit { stsline }, evnt),
                })
            }
            (Inner::Command { mut cmdline }, evnt) => {
                let evnt = cmdline.on_event(app, evnt)?;
                let (inner, evnt) = match evnt {
                    Event::Esc => {
                        let stsline = App::new_stsline(app.coord);
                        (Inner::Edit { stsline }, Event::Noop)
                    }
                    evnt @ Event::Cmd(_, _) => {
                        let stsline = App::new_stsline(app.coord);
                        (Inner::Edit { stsline }, evnt)
                    }
                    evnt => (Inner::Command { cmdline }, evnt),
                };
                Ok((inner, evnt))
            }
            (Inner::None, evnt) => Ok((Inner::None, evnt)),
        }
    }

    fn on_refresh(mut self, app: &mut App) -> Result<Inner> {
        match &mut self {
            Inner::Edit { stsline } => stsline.on_refresh(app)?,
            Inner::AnyKey { prompts, stsline } => {
                prompts[0].on_refresh(app)?;
                stsline.on_refresh(app)?;
            }
            Inner::Command { cmdline } => cmdline.on_refresh(app)?,
            Inner::None => (),
        }
        Ok(self)
    }
}
