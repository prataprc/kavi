use crossterm::{cursor as term_cursor, queue};
#[allow(unused_imports)]
use log::trace;

use std::{convert::TryInto, fmt, io, mem, result};

use crate::{
    buffer::{self, Buffer},
    code::{self, cmd, keymap::Keymap},
    colors::ColorScheme,
    event::Event,
    location::Location,
    syntax::{self, Syntax},
    term::Spanline,
    window::{Coord, Cursor, Render, WinBuffer, Window},
    Error, Result,
};

pub struct WindowCmd {
    coord: Coord,
    cursor: Cursor,
    obc_xy: buffer::Cursor,
    buffer: Buffer,
    syn: syntax::Type,
    scheme: ColorScheme,
    keymap: Keymap,
}

impl fmt::Display for WindowCmd {
    fn fmt(&self, f: &mut fmt::Formatter) -> result::Result<(), fmt::Error> {
        write!(
            f,
            "WindowCmd<{}@{} {}>",
            self.cursor, self.coord, self.obc_xy
        )
    }
}

impl WindowCmd {
    #[inline]
    pub fn new(coord: Coord, app: &code::Code) -> WindowCmd {
        use crate::code::view::NoWrap;

        let buf = {
            let loc = Location::new_ted("code-cmd", io::empty()).unwrap();
            let mut buf = Buffer::from_reader(io::empty(), loc).unwrap();
            buf.mode_insert();
            buf
        };
        let cursor = NoWrap::initial_cursor(false /*line_number*/);
        let obc_xy = (0, 0).into();
        let scheme = app.to_color_scheme(None);
        let syn_code_cmd = syntax::CodeCmd::new("", &scheme).unwrap();
        WindowCmd {
            coord,
            cursor,
            obc_xy,
            buffer: buf,
            syn: syntax::Type::CodeCmd(syn_code_cmd),
            scheme,
            keymap: Keymap::new_cmd(),
        }
    }
}

impl Window for WindowCmd {
    type App = code::Code;

    #[inline]
    fn to_cursor(&self) -> Cursor {
        self.coord.to_top_left() + self.cursor
    }

    fn on_event(&mut self, app: &mut code::Code, mut evnt: Event) -> Result<Event> {
        use crate::code::cmd::Command;

        let mut buf = mem::replace(&mut self.buffer, Default::default());
        evnt = match self.keymap.fold(&mut buf, evnt)? {
            Event::Enter => {
                let line = buf.to_string();
                let syn = mem::replace(&mut self.syn, Default::default());
                match line.split(' ').next() {
                    Some(name) => {
                        let name = name.to_string();
                        let mut val: cmd::Cmd = (name, line, syn).try_into()?;
                        let mut evnt = val.on_command(app)?;
                        evnt.push(Event::Esc);
                        evnt
                    }
                    None => Event::Esc,
                }
            }
            evnt => {
                let evnt = buf.on_event(evnt)?;
                self.syn.on_edit(&buf, evnt)?
            }
        };
        self.buffer = buf;
        Ok(evnt)
    }

    fn on_refresh(&mut self, _app: &mut code::Code) -> Result<()> {
        use crate::code::view::NoWrap;

        let (col, row) = self.coord.to_origin_cursor();
        err_at!(Fatal, termqu!(term_cursor::MoveTo(col, row)))?;

        let mut v = NoWrap::new("cmd", self.coord, self.cursor, self.obc_xy);
        v.set_scroll_off(0).set_line_number(false);
        self.cursor = v.render(&self.buffer, self, &self.scheme)?;
        self.obc_xy = self.buffer.to_xy_cursor();

        Ok(())
    }
}

impl Render for WindowCmd {
    fn to_span_line(&self, buf: &Buffer, a: usize, z: usize) -> Result<Spanline> {
        self.syn.to_span_line(buf, a, z)
    }
}
