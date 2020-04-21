use lazy_static::lazy_static;
use ropey::{self, Rope};
use unicode_width::UnicodeWidthChar;

use std::{
    cell::{self, RefCell},
    convert::TryFrom,
    ffi, fmt, io,
    ops::Bound,
    rc::{self, Rc},
    result,
    sync::Mutex,
};

use crate::{
    config::Config,
    event::Event,
    {err_at, Error, Result},
};

const NEW_LINE_CHAR: char = '\n';

// Buffer mode.
#[derive(Clone)]
pub enum Mode {
    Normal,
    Insert,
}

impl TryFrom<String> for Mode {
    type Error = Error;

    fn try_from(s: String) -> Result<Mode> {
        match s.as_str() {
            "normal" => Ok(Mode::Normal),
            "insert" => Ok(Mode::Insert),
            mode => err_at!(FailConvert, msg: format!("invalid mode {}", mode)),
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter) -> result::Result<(), fmt::Error> {
        match self {
            Mode::Normal => write!(f, "normal"),
            Mode::Insert => write!(f, "insert"),
        }
    }
}

// all bits and pieces of content is managed by buffer.
#[derive(Clone)]
pub struct Buffer {
    mode: Mode,

    location: Location,
    config: Config,
    read_only: bool,

    change: Rc<RefCell<Change>>,
}

impl Default for Buffer {
    fn default() -> Buffer {
        Buffer {
            location: Default::default(),
            config: Default::default(),
            read_only: false,

            mode: Mode::Normal,
            change: Default::default(),
        }
    }
}

impl Buffer {
    pub fn from_reader<R>(data: R, config: Config) -> Result<Buffer>
    where
        R: io::Read,
    {
        let buf = err_at!(FailBuffer, Rope::from_reader(data))?;
        Ok(Buffer {
            location: Default::default(),
            config,
            read_only: false,

            mode: Mode::Normal,
            change: Change::start(buf),
        })
    }

    pub fn empty(config: Config) -> Result<Buffer> {
        let buf = vec![];
        Self::from_reader(buf.as_slice(), config)
    }

    pub fn set_read_only(&mut self, read_only: bool) -> &mut Self {
        self.read_only = read_only;
        self
    }

    pub fn set_location(&mut self, loc: Location) -> &mut Self {
        self.location = loc;
        self
    }

    pub fn set_cursor(&mut self, cursor: usize) -> &mut Self {
        self.as_mut_change().set_cursor(cursor);
        self
    }

    pub fn as_change(&self) -> cell::Ref<Change> {
        self.change.as_ref().borrow()
    }

    pub fn as_mut_change(&mut self) -> cell::RefMut<Change> {
        self.change.as_ref().borrow_mut()
    }
}

impl Buffer {
    pub fn to_string(&self) -> String {
        self.as_change().as_ref().to_string()
    }

    pub fn to_location(&self) -> Location {
        self.location.clone()
    }

    pub fn to_id(&self) -> String {
        match &self.location {
            Location::Anonymous(s) => s.clone(),
            Location::Disk(s) => s.to_str().unwrap().to_string(),
        }
    }

    pub fn to_lines<'a>(
        &'a self,
        from: Bound<usize>,
        to: Bound<usize>,
    ) -> impl Iterator<Item = (usize, String)> + 'a {
        TabfixIter {
            change: self.as_change(),
            from,
            to,
            tabstop: self.config.tabstop.clone(),
        }
    }
}

impl Buffer {
    pub fn visual_cursor(&self) -> (usize, usize) {
        self.as_change().visual_cursor(&self.config.tabstop)
    }

    pub fn to_cursor(&self) -> usize {
        self.as_change().to_cursor()
    }

    pub fn to_xy_cursor(&self) -> (usize, usize) {
        self.as_change().visual_cursor(&self.config.tabstop)
    }

    pub fn handle_event(&mut self, evnt: Event) -> Result<Option<Event>> {
        match self.mode {
            Mode::Normal => self.handle_normal_event(evnt),
            Mode::Insert => self.handle_insert_event(evnt),
        }
    }

    fn handle_normal_event(&mut self, evnt: Event) -> Result<Option<Event>> {
        use Event::{Char, Insert};

        match evnt.clone() {
            Insert => {
                self.mode = Mode::Insert;
                Ok(None)
            }
            Char('i', m) if m.is_empty() => {
                self.mode = Mode::Insert;
                Ok(None)
            }
            _ => Ok(Some(evnt)),
        }
    }

    fn handle_insert_event(&mut self, evnt: Event) -> Result<Option<Event>> {
        use Event::{BackTab, Backspace, Char, Delete, Down, End, Enter};
        use Event::{Esc, Home, Insert, Left, Noop, PageDown, PageUp};
        use Event::{Right, Tab, Up, F};

        match evnt.clone() {
            Char(ch, _) => {
                self.change = Change::to_next_change(&mut self.change);
                self.as_mut_change().insert_char(ch);
                Ok(None)
            }
            Backspace => {
                self.change = Change::to_next_change(&mut self.change);
                self.as_mut_change().backspace();
                Ok(None)
            }
            Enter => {
                self.change = Change::to_next_change(&mut self.change);
                self.as_mut_change().insert_char(NEW_LINE_CHAR);
                Ok(None)
            }
            Tab => {
                self.change = Change::to_next_change(&mut self.change);
                self.as_mut_change().insert_char('\t');
                Ok(None)
            }
            Delete => {
                self.change = Change::to_next_change(&mut self.change);
                self.as_mut_change().remove_at();
                Ok(None)
            }
            Left => {
                self.as_mut_change().move_left();
                Ok(None)
            }
            Right => {
                self.as_mut_change().move_right();
                Ok(None)
            }
            Up => {
                self.as_mut_change().move_up();
                Ok(None)
            }
            Down => {
                self.as_mut_change().move_down();
                Ok(None)
            }
            Home => {
                self.as_mut_change().home();
                Ok(None)
            }
            End => {
                self.as_mut_change().end();
                Ok(None)
            }
            Esc => {
                self.mode = Mode::Normal;
                Ok(None)
            }
            F(_, _) => Ok(Some(evnt)),
            BackTab | Insert | PageUp | PageDown | Noop => Ok(Some(evnt)),
            _ => todo!(),
        }
    }
}

#[derive(Clone)]
pub struct Change {
    buf: Rope,
    parent: Option<rc::Weak<RefCell<Change>>>,
    children: Vec<Rc<RefCell<Change>>>,
    cursor: usize,
}

impl Default for Change {
    fn default() -> Change {
        let bytes: Vec<u8> = vec![];

        Change {
            buf: Rope::from_reader(bytes.as_slice()).unwrap(),
            parent: None,
            children: Default::default(),
            cursor: 0,
        }
    }
}

impl From<Rope> for Change {
    fn from(buf: Rope) -> Change {
        Change {
            buf,
            parent: None,
            children: Default::default(),
            cursor: 0,
        }
    }
}

impl AsRef<Rope> for Change {
    fn as_ref(&self) -> &Rope {
        &self.buf
    }
}

impl AsMut<Rope> for Change {
    fn as_mut(&mut self) -> &mut Rope {
        &mut self.buf
    }
}

impl Change {
    fn start(buf: Rope) -> Rc<RefCell<Change>> {
        Rc::new(RefCell::new(Change {
            buf,
            parent: None,
            children: Default::default(),
            cursor: 0,
        }))
    }

    fn to_next_change(prev: &mut Rc<RefCell<Change>>) -> Rc<RefCell<Change>> {
        let next = Rc::new(RefCell::new(Change {
            buf: prev.borrow().as_ref().clone(),
            parent: None,
            children: Default::default(),
            cursor: prev.borrow().cursor,
        }));

        next.borrow_mut().children.push(Rc::clone(prev));
        prev.borrow_mut().parent = Some(Rc::downgrade(&next));

        next
    }

    fn visual_cursor(&self, tabstop: &str) -> (usize, usize) {
        let tabstop = tabstop.len(); // TODO: account for unicode
        let row_at = self.buf.char_to_line(self.cursor);
        let col_at = self.cursor - self.buf.line_to_char(row_at);
        match self.buf.lines_at(row_at).next() {
            Some(line) => {
                let a_col_at: usize = line
                    .to_string()
                    .chars()
                    .take(col_at)
                    .map(|ch| match ch {
                        '\t' => tabstop,
                        ch => ch.width().unwrap(),
                    })
                    .sum();
                (a_col_at, row_at)
            }
            None => (col_at, row_at),
        }
    }

    pub fn to_cursor(&self) -> usize {
        self.cursor
    }

    pub fn to_xy_cursor(&self) -> (usize, usize) {
        let row_at = self.buf.char_to_line(self.cursor);
        let col_at = self.cursor - self.buf.line_to_char(row_at);
        (col_at, row_at)
    }
}

impl Change {
    fn set_cursor(&mut self, cursor: usize) -> &mut Self {
        self.cursor = cursor;
        self
    }

    fn insert_char(&mut self, ch: char) {
        self.buf.insert_char(self.cursor, ch);
        self.cursor += 1;
    }

    fn backspace(&mut self) {
        if self.cursor > 0 {
            self.buf.remove(self.cursor..=self.cursor);
            self.cursor -= 1;
        }
    }

    fn remove_at(&mut self) {
        if self.cursor < self.buf.len_chars() {
            self.buf.remove(self.cursor..=self.cursor);
        }
    }
}

impl Change {
    fn move_left(&mut self) {
        match self.buf.chars().prev() {
            Some(_) => self.cursor -= 1,
            None => (),
        }
    }

    fn move_right(&mut self) {
        match self.buf.chars().next() {
            Some(_) => self.cursor += 1,
            None => (),
        }
    }

    fn move_up(&mut self) {
        let line_idx = self.buf.char_to_line(self.cursor);
        self.cursor = match self.to_lines().prev() {
            Some(_) => {
                let a_char = self.buf.line_to_char(line_idx - 1);
                a_char + self.to_col()
            }
            None => self.cursor,
        }
    }

    fn move_down(&mut self) {
        let line_idx = self.buf.char_to_line(self.cursor);
        self.cursor = match self.to_lines().next() {
            Some(_) => {
                let a_char = self.buf.line_to_char(line_idx - 1);
                a_char + self.to_col()
            }
            None => self.cursor,
        }
    }

    fn home(&mut self) {
        self.cursor = self.buf.line_to_char(self.buf.char_to_line(self.cursor));
    }

    fn end(&mut self) {
        let mut iter = self.buf.chars();
        for ch in iter.next() {
            if ch == NEW_LINE_CHAR {
                break;
            }
            self.cursor += 1;
        }
    }
}

impl Change {
    fn to_lines(&self) -> ropey::iter::Lines {
        let line_idx = self.buf.char_to_line(self.cursor);
        self.buf.lines_at(line_idx)
    }

    fn to_col(&self) -> usize {
        let a_char = self.buf.line_to_char(self.buf.char_to_line(self.cursor));
        self.cursor - a_char
    }
}

struct TabfixIter<'a> {
    change: cell::Ref<'a, Change>,
    from: Bound<usize>,
    to: Bound<usize>,
    tabstop: String,
}

impl<'a> Iterator for TabfixIter<'a> {
    type Item = (usize, String);

    fn next(&mut self) -> Option<Self::Item> {
        use std::ops::Bound::{Included, Unbounded};

        let r: &Rope = self.change.as_ref();
        let n_lines = r.len_lines();
        match (self.from, self.to) {
            (Included(from), Unbounded) if from < n_lines => {
                // TODO: can this replace be made in-place
                self.from = Included(from + 1);
                let l = r.line(from).to_string().replace('\t', &self.tabstop);
                Some((from + 1, l))
            }
            (Included(from), Included(to)) if from < n_lines && from <= to => {
                self.from = Included(from + 1);
                // TODO: can this replace be made in-place
                let l = r.line(from).to_string().replace('\t', &self.tabstop);
                Some((from + 1, l))
            }
            _ => None,
        }
    }
}

// Location of buffer's content, typically a persistent medium.
#[derive(Clone)]
pub enum Location {
    Anonymous(String),
    Disk(ffi::OsString),
}

lazy_static! {
    static ref ANONYMOUS_COUNT: Mutex<usize> = Mutex::new(0);
}

impl Location {
    fn new_anonymous() -> Location {
        let mut count = ANONYMOUS_COUNT.lock().unwrap();
        *count = *count + 1;
        Location::Anonymous(format!("anonymous-{}", count))
    }

    fn new_disk(loc: &ffi::OsStr) -> Location {
        Location::Disk(loc.to_os_string())
    }
}

impl Default for Location {
    fn default() -> Location {
        Location::new_anonymous()
    }
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter) -> result::Result<(), fmt::Error> {
        match self {
            Location::Anonymous(s) => write!(f, "{}", s),
            Location::Disk(s) => {
                let s = s.clone().into_string().unwrap();
                write!(f, "{}", s)
            }
        }
    }
}
