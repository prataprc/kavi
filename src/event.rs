//! Module implements and abstracts all events across ted applications.
//! Some events are generated by user, a keystrokes and mouse movements.
//! Other events are created by application's `keymap` or application
//! components.

use crossterm::event::{Event as TermEvent, KeyCode, KeyEvent, KeyModifiers};
#[allow(unused_imports)]
use log::debug;
use tree_sitter as ts;
use unicode_width::UnicodeWidthChar;

use std::{fmt, iter::FromIterator, mem, result};

use crate::{
    buffer::{self, Buffer},
    mark,
    pubsub::Notify,
    window::{WinBuffer, WindowLess, WindowPrompt},
    Error, Result,
};

/// Events
#[derive(Clone, Eq, PartialEq)]
pub enum Event {
    // Direct key events
    Backspace(KeyModifiers),
    Enter(KeyModifiers),
    Tab(KeyModifiers),
    Delete(KeyModifiers),
    Char(char, KeyModifiers),
    FKey(u8, KeyModifiers),
    InsKey(KeyModifiers),
    Left(KeyModifiers),
    Right(KeyModifiers),
    Up(KeyModifiers),
    Down(KeyModifiers),
    Home(KeyModifiers),
    End(KeyModifiers),
    PageUp(KeyModifiers),
    PageDown(KeyModifiers),
    BackTab,
    Esc,
    // prefix events
    N(usize),     // Num-prefix (n,)
    G(usize),     // Global     (n,)
    B(usize, DP), // Bracket    (n, Left/Right)
    F(usize, DP), // Find-char  (n, Left/Right)
    T(usize, DP), // Till-char  (n, Left/Right)
    J(char),      // jump prefix (['`],)
    Z(usize),     // scroll prefix (n,)
    M,            // mark prefix
    Op(Opr),      // Operation  (op-event)
    // folded events for buffer management.
    Mt(Mto),        // Motion     (n, motion-event)
    Mr(mark::Mark), // (mark-value,)
    Md(Mod),        // modal command.
    Wr(Cud),        // insert command.
    TabInsert(String),
    TabClear,
    // other events
    Appn(Appn),
    JumpFrom(usize), // (cursor,)
    // local events
    Edit(Edit),
    List(Vec<Event>),
    Notify(Notify),
    Noop,
}

impl Event {
    /// Return the keystroke modifiers like ctrl, alt, etc.. or empty if the
    /// keystore do not contain any modifiers.
    pub fn to_modifiers(&self) -> KeyModifiers {
        use Event::*;

        let empty = KeyModifiers::empty();

        match self.clone() {
            Backspace(m) | Enter(m) | Tab(m) | Delete(m) => m,
            Char(_, m) | FKey(_, m) | InsKey(m) => m,
            Left(m) | Right(m) | Up(m) | Down(m) => m,
            Home(m) | End(m) | PageUp(m) | PageDown(m) => m,
            BackTab | Esc => empty,
            // prefix events
            N(_) | G(_) | B(_, _) | F(_, _) | T(_, _) | M | J(_) | Z(_) => empty,
            Op(op) => op.to_modifiers(),
            // folded events for buffer management.
            Mr(_) => empty,
            Md(mode) => mode.to_modifiers(),
            Mt(mto) => mto.to_modifiers(),
            Wr(cud) => cud.to_modifiers(),
            TabInsert(_) | TabClear => empty,
            // other events
            Appn(_) | JumpFrom(_) => empty,
            // local events
            Edit(_) | List(_) | Notify(_) | Noop => empty,
        }
    }

    /// Return whether control modifier is part of the keystroke.
    pub fn is_control(&self) -> bool {
        match self {
            Event::FKey(_, m) => m.contains(KeyModifiers::CONTROL),
            Event::Char(_, m) => m.contains(KeyModifiers::CONTROL),
            _ => false,
        }
    }

    /// Return whether the event is to enter insert/append/open/replace mode.
    /// In short, whether shift to insert-mode.
    pub fn is_insert(&self) -> bool {
        use {
            Event::Md,
            Mod::{Append, Insert, Open},
        };

        match self {
            Md(Insert(_, _)) | Md(Append(_, _)) | Md(Open(_, _)) => true,
            _ => false,
        }
    }

    /// Push another event into the current event. Events can also act as a
    /// FIFO. This is useful when more events are accumulated as it gets
    /// processed across the pipeline.
    pub fn push(&mut self, evnt: Event) {
        *self = match (self.clone(), evnt) {
            (old_evnt, Event::Noop) => old_evnt,
            (Event::Noop, evnt) => evnt,
            (Event::List(mut evnts), evnt) => {
                evnts.push(evnt);
                Event::List(evnts)
            }
            (old_evnt, evnt) => Event::List(vec![old_evnt, evnt]),
        };
    }

    /// Pop from list of events. Events can also act as a FIFO. This is
    /// useful when more events are accumulated as it gets processed
    /// across the pipeline.
    pub fn pop(&mut self) -> Event {
        match self {
            Event::Noop => Event::Noop,
            Event::List(events) => match events.pop() {
                Some(evnt) => evnt,
                None => {
                    *self = Event::Noop;
                    Event::Noop
                }
            },
            _ => {
                let evnt = self.clone();
                *self = Event::Noop;
                evnt
            }
        }
    }

    pub fn drain(&mut self) {
        *self = Event::Noop
    }
}

impl Iterator for Event {
    type Item = Event;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Event::Noop => None,
            Event::List(evnts) if evnts.len() > 0 => Some(evnts.remove(0)),
            Event::List(_) => None,
            _ => {
                let evnt = mem::replace(self, Event::Noop);
                Some(evnt)
            }
        }
    }
}

impl FromIterator<Event> for Event {
    fn from_iter<T>(iter: T) -> Event
    where
        T: IntoIterator<Item = Event>,
    {
        let mut evnts: Vec<Event> = vec![];
        for evnt in iter {
            match Event::from_iter(evnt) {
                Event::List(es) => evnts.extend(es),
                Event::Noop => (),
                evnt => evnts.push(evnt),
            }
        }
        match evnts.len() {
            0 => Event::Noop,
            1 => evnts.remove(0),
            _ => Event::List(evnts),
        }
    }
}
impl Extend<Event> for Event {
    fn extend<I>(&mut self, iter: I)
    where
        I: IntoIterator<Item = Event>,
    {
        let mut evnts: Vec<Event> = iter.into_iter().collect();
        let evnts = match mem::replace(self, Event::default()) {
            Event::List(mut events) => {
                events.extend(evnts);
                events
            }
            Event::Noop => evnts,
            evnt => {
                evnts.insert(0, evnt);
                evnts
            }
        };
        *self = if evnts.len() > 0 {
            Event::List(evnts)
        } else {
            Event::Noop
        };
    }
}

impl fmt::Display for Event {
    fn fmt(&self, f: &mut fmt::Formatter) -> result::Result<(), fmt::Error> {
        use Event::*;

        match self {
            // insert events
            Backspace(_) => write!(f, "backspace"),
            Enter(_) => write!(f, "enter"),
            Tab(_) => write!(f, "tab"),
            Delete(_) => write!(f, "delete"),
            Char(ch, m) => write!(f, "char({:?},{:?})", ch, m),
            FKey(ch, _) => write!(f, "fkey({})", ch),
            InsKey(_) => write!(f, "insert"),
            Left(_) => write!(f, "left"),
            Right(_) => write!(f, "right"),
            Up(_) => write!(f, "up"),
            Down(_) => write!(f, "down"),
            Home(_) => write!(f, "home"),
            End(_) => write!(f, "end"),
            PageUp(_) => write!(f, "page-up"),
            PageDown(_) => write!(f, "page-down"),
            BackTab => write!(f, "backtab"),
            Esc => write!(f, "esc"),
            // prefix events
            N(n) => write!(f, "n({})", n),
            G(n) => write!(f, "g({})", n),
            B(n, dp) => write!(f, "b({},{})", n, dp),
            F(n, dp) => write!(f, "f({},{})", n, dp),
            T(n, dp) => write!(f, "t({},{})", n, dp),
            M => write!(f, "m"),
            J(ch) => write!(f, "j({})", ch),
            Z(n) => write!(f, "z({})", n),
            Op(opr) => write!(f, "op({})", opr),
            // folded events for buffer management.
            Mr(mark) => write!(f, "mark({})", mark),
            Mt(mt) => write!(f, "mt({})", mt),
            Md(mode) => write!(f, "md({})", mode),
            Wr(cud) => write!(f, "wr({})", cud),
            TabInsert(_) => write!(f, "tab-insert"),
            TabClear => write!(f, "tab-clear"),
            // other events
            Appn(cd) => write!(f, "Appn({})", cd),
            JumpFrom(cursor) => write!(f, "jump-from({})", cursor),
            // local events
            Edit(val) => write!(f, "edit({})", val),
            List(es) => write!(f, "list({})", es.len()),
            Notify(notf) => write!(f, "notify({})", notf),
            Noop => write!(f, "noop"),
        }
    }
}

impl From<TermEvent> for Event {
    fn from(evnt: TermEvent) -> Event {
        match evnt {
            TermEvent::Key(KeyEvent { code, modifiers: m }) => {
                let empty = m.is_empty();
                match code {
                    //
                    KeyCode::Backspace => Event::Backspace(m),
                    KeyCode::Enter => Event::Enter(m),
                    KeyCode::Tab => Event::Tab(m),
                    KeyCode::Delete => Event::Delete(m),
                    KeyCode::Char(ch) => Event::Char(ch, m),
                    KeyCode::F(f) if empty => Event::FKey(f, m),
                    KeyCode::BackTab => Event::BackTab,
                    KeyCode::Esc => Event::Esc,
                    KeyCode::Insert => Event::InsKey(m),
                    KeyCode::Left if empty => Event::Left(m),
                    KeyCode::Right if empty => Event::Right(m),
                    KeyCode::Up if empty => Event::Up(m),
                    KeyCode::Down if empty => Event::Down(m),
                    KeyCode::Home if empty => Event::Home(m),
                    KeyCode::End if empty => Event::End(m),
                    KeyCode::PageUp if empty => Event::PageUp(m),
                    KeyCode::PageDown if empty => Event::PageDown(m),
                    KeyCode::Null => Event::Noop,
                    _ => Event::Noop,
                }
            }
            _ => Event::Noop,
        }
    }
}

impl Default for Event {
    fn default() -> Event {
        Event::Noop
    }
}

impl From<Vec<Event>> for Event {
    fn from(evnts: Vec<Event>) -> Event {
        let mut out: Vec<Event> = vec![];
        for evnt in evnts.into_iter() {
            match evnt {
                Event::List(es) => out.extend(es.into_iter()),
                e => out.push(e),
            }
        }
        Event::List(out)
    }
}

impl From<Event> for Vec<Event> {
    fn from(evnt: Event) -> Vec<Event> {
        match evnt {
            Event::List(evnts) => evnts,
            evnt => vec![evnt],
        }
    }
}

/// Event argument, specify the edit operation performed in buffer.
#[derive(Clone, Eq, PartialEq)]
pub enum Edit {
    /// Insert new `txt` into buffer at `cursor`.
    Ins { cursor: usize, txt: String },
    /// Delete `txt` found at `cursor`.
    Del { cursor: usize, txt: String },
    /// Replace `oldt` found at `cursor` with `newt`.
    Chg {
        cursor: usize,
        oldt: String,
        newt: String,
    },
}

impl fmt::Display for Edit {
    fn fmt(&self, f: &mut fmt::Formatter) -> result::Result<(), fmt::Error> {
        match self {
            Edit::Ins { cursor, txt } => {
                let n = txt.len();
                write!(f, "ins<{}@{}>", cursor, n)
            }
            Edit::Del { cursor, txt } => {
                let n = txt.len();
                write!(f, "del<{}@{}>", cursor, n)
            }
            Edit::Chg { cursor, oldt, newt } => {
                let (n, m) = (oldt.len(), newt.len());
                write!(f, "chg<{}->{}@{}", n, m, cursor)
            }
        }
    }
}

impl fmt::Debug for Edit {
    fn fmt(&self, f: &mut fmt::Formatter) -> result::Result<(), fmt::Error> {
        <Self as fmt::Display>::fmt(self, f)
    }
}

impl Edit {
    pub fn into_ts_input(self, buf: &Buffer) -> Result<ts::InputEdit> {
        let (st, oe, ne) = match self {
            Edit::Ins { cursor, txt } => {
                let n = txt.chars().count();
                (cursor, cursor, cursor + n)
            }
            Edit::Del { cursor, txt } => {
                let n = txt.chars().count();
                (cursor, cursor + n, cursor)
            }
            Edit::Chg { cursor, oldt, newt } => {
                let m = oldt.chars().count();
                let n = newt.chars().count();
                (cursor, cursor + m, cursor + n)
            }
        };
        Ok(ts::InputEdit {
            start_byte: Self::to_byte(st, buf),
            old_end_byte: Self::to_byte_end(oe, buf)?,
            new_end_byte: Self::to_byte_end(ne, buf)?,
            start_position: {
                let (row, column) = Self::to_xy(st, buf);
                ts::Point { row, column }
            },
            old_end_position: {
                let (row, column) = Self::to_xy_end(st, buf)?;
                ts::Point { row, column }
            },
            new_end_position: {
                let (row, column) = Self::to_xy_end(st, buf)?;
                ts::Point { row, column }
            },
        })
    }

    fn to_byte(cursor: usize, buf: &Buffer) -> usize {
        buf.char_to_byte(cursor)
    }

    fn to_byte_end(cursor: usize, buf: &Buffer) -> Result<usize> {
        let m = match buf.chars_at(cursor, DP::Right)?.next() {
            Some(ch) => ch.width().unwrap_or(0).saturating_sub(1),
            None => 0,
        };
        Ok(buf.char_to_byte(cursor) + m)
    }

    fn to_xy(cursor: usize, buf: &Buffer) -> (usize, usize) {
        let buffer::Cursor { row, .. } = buf.to_xy_cursor(Some(cursor));
        let row = buf.char_to_byte(buf.line_to_char(row));
        let col = buf.char_to_byte(cursor).saturating_sub(row);
        (row, col)
    }

    fn to_xy_end(cursor: usize, buf: &Buffer) -> Result<(usize, usize)> {
        let m = match buf.chars_at(cursor, DP::Right)?.next() {
            Some(ch) => ch.width().unwrap_or(0).saturating_sub(1),
            None => 0,
        };
        let buffer::Cursor { row, .. } = buf.to_xy_cursor(Some(cursor));
        let row = buf.char_to_byte(buf.line_to_char(row));
        let col = buf.char_to_byte(cursor).saturating_sub(row);
        Ok((row, col + m))
    }
}
/// Event argument, specify the direction or position.
#[derive(Clone, Copy, Eq, PartialEq, PartialOrd)]
pub enum DP {
    /// Left-Direction to move/operate.
    Left,
    /// Right-Direction to move/operate.
    Right,
    Start,
    End,
    /// Motion/operation bound by end-of-line.
    LineBound,
    /// Motion/operation not-bound by end-of-line.
    Nobound,
    /// Non-Blank column in the line, as in, first non-blank / last non-blank.
    TextCol,
    /// Cursor sticks to current-col, for subsequent linewise motion/operation,
    /// until next characterwise motion/operation.
    StickyCol,
    None,
}

impl fmt::Display for DP {
    fn fmt(&self, f: &mut fmt::Formatter) -> result::Result<(), fmt::Error> {
        match self {
            DP::Left => write!(f, "left"),
            DP::Right => write!(f, "right"),
            DP::Start => write!(f, "start"),
            DP::End => write!(f, "end"),
            DP::LineBound => write!(f, "line_bound"),
            DP::Nobound => write!(f, "no_bound"),
            DP::TextCol => write!(f, "TextCol"),
            DP::StickyCol => write!(f, "sticky_col"),
            DP::None => write!(f, "nope"),
        }
    }
}

impl fmt::Debug for DP {
    fn fmt(&self, f: &mut fmt::Formatter) -> result::Result<(), fmt::Error> {
        <Self as fmt::Display>::fmt(self, f)
    }
}

impl DP {
    fn dir_xor(self, rhs: Self) -> Result<Self> {
        let dp = match (self, rhs) {
            (DP::Left, DP::Right) => DP::Left,
            (DP::Right, DP::Right) => DP::Right,
            (DP::Left, DP::Left) => DP::Right,
            (DP::Right, DP::Left) => DP::Left,
            (x, y) => {
                let msg = format!("invalid direction {} {}", x, y);
                err_at!(Fatal, msg: msg)?
            }
        };
        Ok(dp)
    }
}

/// Event argument specify the text operation.
#[derive(Clone, Eq, PartialEq)]
pub enum Opr {
    Change(usize, Mto),    // (n, motion-command)
    Delete(usize, Mto),    // (n, motion-command)
    Yank(usize, Mto),      // (n, motion-command)
    Swapcase(usize, Mto),  // (n, motion-command)
    Lowercase(usize, Mto), // (n, motion-command)
    Uppercase(usize, Mto), // (n, motion-command)
    Filter(usize, Mto),    // (n, motion-command)
    Equal(usize, Mto),     // (n, motion-command)
    Format(usize, Mto),    // (n, motion-command)
    Encode(usize, Mto),    // (n, motion-command)
    RShift(usize, Mto),    // (n, motion-command)
    LShift(usize, Mto),    // (n, motion-command)
    Fold(usize, Mto),      // (n, motion-command)
    Func(usize, Mto),      // (n, motion-command)
}

impl fmt::Display for Opr {
    fn fmt(&self, f: &mut fmt::Formatter) -> result::Result<(), fmt::Error> {
        match self {
            Opr::Change(n, mto) => write!(f, "change({},{})", n, mto),
            Opr::Delete(n, mto) => write!(f, "delete({},{})", n, mto),
            Opr::Yank(n, mto) => write!(f, "yank({},{})", n, mto),
            Opr::Swapcase(n, mto) => write!(f, "swapcase({},{})", n, mto),
            Opr::Lowercase(n, mto) => write!(f, "lowercase({},{})", n, mto),
            Opr::Uppercase(n, mto) => write!(f, "uppercase({},{})", n, mto),
            Opr::Filter(n, mto) => write!(f, "filter({},{})", n, mto),
            Opr::Equal(n, mto) => write!(f, "equal({},{})", n, mto),
            Opr::Format(n, mto) => write!(f, "format({},{})", n, mto),
            Opr::Encode(n, mto) => write!(f, "encode({},{})", n, mto),
            Opr::RShift(n, mto) => write!(f, "rshift({},{})", n, mto),
            Opr::LShift(n, mto) => write!(f, "lshift({},{})", n, mto),
            Opr::Fold(n, mto) => write!(f, "fold({},{})", n, mto),
            Opr::Func(n, mto) => write!(f, "func({},{})", n, mto),
        }
    }
}

impl Opr {
    fn to_modifiers(&self) -> KeyModifiers {
        KeyModifiers::empty()
    }
}

/// Modal command.
#[derive(Clone, Eq, PartialEq)]
pub enum Mod {
    Esc,
    Insert(usize, DP),  // (n, None/TextCol)
    Append(usize, DP),  // (n, Right/End)
    Replace(usize, DP), // (n, None/TextCol)
    Open(usize, DP),    // (n, Left/Right)
}

impl fmt::Display for Mod {
    fn fmt(&self, f: &mut fmt::Formatter) -> result::Result<(), fmt::Error> {
        match self {
            Mod::Esc => write!(f, "esc"),
            Mod::Insert(n, dp) => write!(f, "insert({},{})", n, dp),
            Mod::Append(n, dp) => write!(f, "append({},{})", n, dp),
            Mod::Replace(n, dp) => write!(f, "replace({},{})", n, dp),
            Mod::Open(n, dp) => write!(f, "open({},{})", n, dp),
        }
    }
}

impl Mod {
    fn to_modifiers(&self) -> KeyModifiers {
        KeyModifiers::empty()
    }
}

/// Insert command.
#[derive(Clone, Eq, PartialEq)]
pub enum Cud {
    Backspace(usize), // remove n chars before cursor
    Delete(usize),    // remove n chars forward from cursor.
    Tab(usize),       // insert tab/spaces
    Enter(usize),     // insert newline(s)
    Char(char),       // insert char
    ReInsert,
    RemoveWord,
    RemoveLine,
    NextWord,
    PrevWord,
    RShift(usize), // (n,)
    LShift(usize), // (n,)
}

impl fmt::Display for Cud {
    fn fmt(&self, f: &mut fmt::Formatter) -> result::Result<(), fmt::Error> {
        match self {
            Cud::Backspace(n) => write!(f, "backspace({})", n),
            Cud::Delete(n) => write!(f, "delete({})", n),
            Cud::Tab(n) => write!(f, "tab({})", n),
            Cud::Enter(n) => write!(f, "enter({})", n),
            Cud::Char(ch) => write!(f, "char({})", ch),
            Cud::ReInsert => write!(f, "re-insert"),
            Cud::RemoveWord => write!(f, "remove-word"),
            Cud::RemoveLine => write!(f, "remove-line"),
            Cud::NextWord => write!(f, "next-line"),
            Cud::PrevWord => write!(f, "prev-line"),
            Cud::RShift(n) => write!(f, "rshift({})", n),
            Cud::LShift(n) => write!(f, "lshift({})", n),
        }
    }
}

impl Cud {
    fn to_modifiers(&self) -> KeyModifiers {
        KeyModifiers::empty()
    }
}
/// Scroll sub-commands for Mto motion command.
#[derive(Clone, Eq, PartialEq)]
pub enum Scroll {
    // vertical scrolls
    Ones,
    Lines,
    Pages,
    Cursor,
    TextUp,
    TextCenter,
    TextBottom,
    // horizontal scrolls
    Chars,
    Slide,
    Align,
}

impl fmt::Display for Scroll {
    fn fmt(&self, f: &mut fmt::Formatter) -> result::Result<(), fmt::Error> {
        match self {
            Scroll::Ones => write!(f, "once"),
            Scroll::Lines => write!(f, "lines"),
            Scroll::Pages => write!(f, "pages"),
            Scroll::Cursor => write!(f, "cursor"),
            Scroll::TextUp => write!(f, "text-up"),
            Scroll::TextCenter => write!(f, "text-center"),
            Scroll::TextBottom => write!(f, "text-bottom"),
            Scroll::Chars => write!(f, "chars"),
            Scroll::Slide => write!(f, "slide"),
            Scroll::Align => write!(f, "align"),
        }
    }
}

/// Event argument specify the cursor motion.
#[derive(Clone, Eq, PartialEq)]
pub enum Mto {
    // character-wise motion
    Left(usize, DP),       // (n, LineBound/Nobound)
    Right(usize, DP),      // (n, LineBound/Nobound)
    LineHome(DP),          // TextCol/StickyCol/None
    LineEnd(usize, DP),    // (n, TextCol/StickyCol/None)
    LineMiddle(usize, DP), // (n-percent, None)
    ScreenHome(DP),        // TextCol/None
    ScreenEnd(usize, DP),  // (n, TextCol/None)
    ScreenMiddle,
    Col(usize),                     // (n,)
    CharF(usize, Option<char>, DP), // (n, ch, Left/Right)
    CharT(usize, Option<char>, DP), // (n, ch, Left/Right)
    CharR(usize, DP),               // repeat CharF/CharT (n, Left/Right)
    // line-wise motion.
    Up(usize, DP),         // (n, TextCol/StickyCol/None)
    Down(usize, DP),       // (n, TextCol/StickyCol/None)
    Row(usize, DP),        // (n, TextCol/None)
    Percent(usize, DP),    // (n, TextCol/None)
    Cursor(usize),         // (n,)
    ScreenUp(usize, DP),   // (n, None)
    ScreenDown(usize, DP), // (n, None)
    // word/sentence/para wise motion
    Word(usize, DP, DP),  // (n, Left/Right, Start/End)
    WWord(usize, DP, DP), // (n, Left/Right, Start/End)
    Sentence(usize, DP),  // (n, Left/Right)
    Para(usize, DP),      // (n, Left/Right)
    // window motion
    WinH(usize),                  // (n,)
    WinM,                         // (n,)
    WinL(usize),                  // (n,)
    WinScroll(usize, Scroll, DP), // (n, Scroll, Left/Right/TextCol/None)
    // other motions
    MatchPair,
    UnmatchPair(usize, char, DP), // (n, marker, Left/Right)
    // jumps and marks
    Jump(char, char), // (['`], [a-zA-Z0-9])

    Bracket(usize, char, char, DP),     // (n, yin, yan, Left/Right)
    Pattern(usize, Option<String>, DP), // (n, pattern, Left/Right)
    PatternR(usize, DP),                // repeat pattern (n, Left/Right)
    None,
}

impl Default for Mto {
    fn default() -> Mto {
        Mto::None
    }
}

impl fmt::Display for Mto {
    fn fmt(&self, f: &mut fmt::Formatter) -> result::Result<(), fmt::Error> {
        match self {
            Mto::Left(n, dp) => write!(f, "left({},{})", n, dp),
            Mto::Right(n, dp) => write!(f, "right({},{})", n, dp),
            Mto::Up(n, dp) => write!(f, "up({},{})", n, dp),
            Mto::Down(n, dp) => write!(f, "down({},{})", n, dp),
            Mto::Col(n) => write!(f, "col({})", n),
            Mto::LineHome(dp) => write!(f, "line-home({})", dp),
            Mto::LineEnd(n, dp) => write!(f, "line-end({},{})", n, dp),
            Mto::LineMiddle(n, dp) => write!(f, "line-middle({},{})", n, dp),
            Mto::ScreenHome(dp) => write!(f, "screen-home({})", dp),
            Mto::ScreenEnd(n, dp) => write!(f, "screen-end({},{})", n, dp),
            Mto::ScreenMiddle => write!(f, "screen-middle"),
            Mto::Row(n, dp) => write!(f, "row({},{})", n, dp),
            Mto::Percent(n, dp) => write!(f, "percent({},{})", n, dp),
            Mto::Cursor(n) => write!(f, "cursor({})", n),
            Mto::ScreenUp(n, dp) => write!(f, "screen-up({},{})", n, dp),
            Mto::ScreenDown(n, dp) => write!(f, "screen-down({},{})", n, dp),
            Mto::CharF(n, ch, dp) => write!(f, "charf({},{:?},{})", n, ch, dp),
            Mto::CharT(n, ch, dp) => write!(f, "chart({},{:?},{})", n, ch, dp),
            Mto::CharR(n, dp) => write!(f, "charr({},{})", n, dp),
            Mto::Word(n, dp1, dp2) => write!(f, "word({},{},{})", n, dp1, dp2),
            Mto::WWord(n, dp1, dp2) => write!(f, "wword({},{},{})", n, dp1, dp2),
            Mto::Sentence(n, dp) => write!(f, "sentence({},{})", n, dp),
            Mto::Para(n, dp) => write!(f, "para({},{})", n, dp),
            Mto::WinH(n) => write!(f, "winh({})", n),
            Mto::WinM => write!(f, "winm"),
            Mto::WinL(n) => write!(f, "winl({})", n),
            Mto::WinScroll(n, scroll, dp) => {
                write!(f, "win-scroll({},{},{})", n, scroll, dp /*text-col*/)
            }
            Mto::MatchPair => write!(f, "match-pair"),
            Mto::UnmatchPair(n, ch, dir) => {
                write!(f, "unmatch-pair({},{},{})", n, ch, dir /* for exprs */)
            }
            Mto::Jump(typ, ch) => write!(f, "mark-jump({},{})", typ, ch),

            Mto::Bracket(n, ch1, ch2, dp) => {
                //
                write!(f, "bracket({},{},{},{})", n, ch1, ch2, dp)
            }
            Mto::Pattern(n, _, dp) => write!(f, "pattern({},{})", n, dp),
            Mto::PatternR(n, dp) => write!(f, "patternr({},{})", n, dp),
            Mto::None => write!(f, "none"),
        }
    }
}

impl Mto {
    /// Do the character/pattern motion in the opposite direction.
    pub fn dir_xor(self, n: usize, new_dp: DP) -> Result<Self> {
        use Mto::{CharF, CharT, Pattern};

        let evnt = match self {
            CharF(_, ch, dp) => CharF(n, ch, dp.dir_xor(new_dp)?),
            CharT(_, ch, dp) => CharT(n, ch, dp.dir_xor(new_dp)?),
            Pattern(_, ch, dp) => Pattern(n, ch, dp.dir_xor(new_dp)?),
            Mto::None => Mto::None,
            _ => err_at!(Fatal, msg: format!("unexpected {}", self))?,
        };
        Ok(evnt)
    }

    fn to_modifiers(&self) -> KeyModifiers {
        KeyModifiers::empty()
    }
}

/// Event specific to application `code`.
#[derive(Clone, Eq, PartialEq)]
pub enum Appn {
    Less(Box<WindowLess>),
    Prompt(Box<WindowPrompt>),
    StatusFile,
    StatusCursor,
}

impl fmt::Display for Appn {
    fn fmt(&self, f: &mut fmt::Formatter) -> result::Result<(), fmt::Error> {
        use Appn::{Less, Prompt, StatusCursor, StatusFile};

        match self {
            Less(_) => write!(f, "less"),
            Prompt(_) => write!(f, "prompt"),
            StatusFile => write!(f, "status_file"),
            StatusCursor => write!(f, "status_cursor"),
        }
    }
}
