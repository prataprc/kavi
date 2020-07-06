//! Module implements and abstracts all events across ted applications.
//! Some events are generated by user, a keystrokes and mouse movements.
//! Other events are created by application's `keymap` or application
//! components.

use crossterm::event::{Event as TermEvent, KeyCode, KeyEvent, KeyModifiers};
use tree_sitter as ts;

use std::{fmt, mem, result};

use crate::{pubsub::Notify, Error, Result};

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
    Insert(KeyModifiers),
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
    // folded events for buffer management.
    N(usize),     // Num-prefix (n,)
    G(usize),     // Global     (n,)
    B(usize, DP), // Bracket    (n, Left/Right)
    F(usize, DP), // Find-char  (n, Left/Right)
    T(usize, DP), // Till-char  (n, Left/Right)
    Op(Opr),      // Operation  (op-event)
    Md(Mod),      // Mode       (n, mode-event)
    Mt(Mto),      // Motion     (n, motion-event)
    // other events
    Edit(Input),
    List(Vec<Event>),
    Notify(Notify),
    Code(Code),
    Ted(Ted),
    Noop,
}

impl Event {
    /// Return the keystroke modifiers like ctrl, alt, etc.. or empty if the
    /// keystore do not contain any modifiers.
    pub fn to_modifiers(&self) -> KeyModifiers {
        match self {
            Event::FKey(_, m) => m.clone(),
            Event::Char(_, m) => m.clone(),
            _ => KeyModifiers::empty(),
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

    /// Return whether the event is to enter insert/append/open mode. In short,
    /// whether shift to insert-mode.
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
        match self {
            Event::List(events) => events.push(evnt),
            Event::Noop => *self = evnt,
            _ => {
                let event = mem::replace(self, Default::default());
                *self = Event::List(vec![event, evnt]);
            }
        }
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

impl Extend<Event> for Event {
    fn extend<I>(&mut self, iter: I)
    where
        I: IntoIterator<Item = Event>,
    {
        let mut evnts: Vec<Event> = iter.into_iter().collect();
        let evnts = match mem::replace(self, Default::default()) {
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
        use Event::Edit;
        use Event::{BackTab, Code, FKey, Insert, List, Noop, Notify};
        use Event::{Backspace, Char, Delete, Enter, Esc, Tab};
        use Event::{Down, End, Home, Left, PageDown, PageUp, Right, Up};
        use Event::{Md, Mt, Op, Ted, B, F, G, N, T};

        match self {
            // insert events
            Backspace(_) => write!(f, "backspace"),
            Enter(_) => write!(f, "enter"),
            Tab(_) => write!(f, "tab"),
            Delete(_) => write!(f, "delete"),
            Char(ch, _) => write!(f, "char({:?})", ch),
            FKey(ch, _) => write!(f, "fkey({})", ch),
            Insert(_) => write!(f, "insert"),
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
            // folded events for buffer management
            B(n, dp) => write!(f, "b({},{})", n, dp),
            G(n) => write!(f, "g({})", n),
            F(n, dp) => write!(f, "f({},{})", n, dp),
            T(n, dp) => write!(f, "t({},{})", n, dp),
            N(n) => write!(f, "b({}", n),
            Op(opr) => write!(f, "op({})", opr),
            Md(md) => write!(f, "md({})", md),
            Mt(mt) => write!(f, "mt({})", mt),
            // other events
            Edit(val) => write!(f, "edit({})", val),
            List(es) => write!(f, "list({})", es.len()),
            Notify(notf) => write!(f, "notify({})", notf),
            Code(cd) => write!(f, "Code({})", cd),
            Ted(td) => write!(f, "Ted({})", td),
            Noop => write!(f, "noop"),
        }
    }
}

impl From<TermEvent> for Event {
    fn from(evnt: TermEvent) -> Event {
        match evnt {
            TermEvent::Key(KeyEvent { code, modifiers: m }) => {
                let ctrl = m.contains(KeyModifiers::CONTROL);
                let empty = m.is_empty();
                match code {
                    //
                    KeyCode::Backspace => Event::Backspace(m),
                    KeyCode::Enter => Event::Enter(m),
                    KeyCode::Tab => Event::Tab(m),
                    KeyCode::Delete => Event::Delete(m),
                    KeyCode::Char('[') if ctrl => Event::Esc,
                    KeyCode::Char(ch) => Event::Char(ch, m),
                    KeyCode::F(f) if empty => Event::FKey(f, m),
                    KeyCode::BackTab => Event::BackTab,
                    KeyCode::Esc => Event::Esc,
                    KeyCode::Insert => Event::Insert(m),
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
/// Event argument, specify the direction or position.
#[derive(Clone, Copy, Eq, PartialEq)]
pub enum DP {
    Left,
    Right,
    Find,
    Till,
    Start,
    End,
    LineBound,
    Nobound,
    Caret,
    TextCol,
    None,
}

impl fmt::Display for DP {
    fn fmt(&self, f: &mut fmt::Formatter) -> result::Result<(), fmt::Error> {
        match self {
            DP::Left => write!(f, "left"),
            DP::Right => write!(f, "right"),
            DP::Find => write!(f, "find"),
            DP::Till => write!(f, "till"),
            DP::Start => write!(f, "start"),
            DP::End => write!(f, "end"),
            DP::LineBound => write!(f, "line_bound"),
            DP::Nobound => write!(f, "no_bound"),
            DP::Caret => write!(f, "caret"),
            DP::TextCol => write!(f, "text_col"),
            DP::None => write!(f, "nope"),
        }
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

/// Ted is a text processing application and like vi, it is modal.
#[derive(Clone, Eq, PartialEq)]
pub enum Mod {
    Esc,
    Insert(usize, DP), // (n, None/Caret)
    Append(usize, DP), // (n, Right/End)
    Open(usize, DP),   // (n, Left/Right)
}

impl fmt::Display for Mod {
    fn fmt(&self, f: &mut fmt::Formatter) -> result::Result<(), fmt::Error> {
        match self {
            Mod::Esc => write!(f, "esc"),
            Mod::Insert(n, dp) => write!(f, "insert({},{})", n, dp),
            Mod::Append(n, dp) => write!(f, "append({},{})", n, dp),
            Mod::Open(n, dp) => write!(f, "open({},{})", n, dp),
        }
    }
}

/// Event argument specify the cursor motion.
#[derive(Clone, Eq, PartialEq)]
pub enum Mto {
    Left(usize, DP),  // (n, LineBound/Nobound)
    Right(usize, DP), // (n, LineBound/Nobound)
    Up(usize, DP),    // (n, Caret/None)
    Down(usize, DP),  // (n, Caret/None)
    Col(usize),       // (n,)
    Home(DP),         // (n, Caret/TextCol/None)
    End,
    Row(usize, DP),                     // (n, Caret/None)
    Percent(usize),                     // (n,)
    Cursor(usize),                      // (n,)
    CharF(usize, Option<char>, DP),     // (n, ch, Left/Right)
    CharT(usize, Option<char>, DP),     // (n, ch, Left/Right)
    CharR(usize, DP),                   // repeat CharF/CharT (n, Left/Right)
    Word(usize, DP, DP),                // (n, Left/Right, Start/End)
    WWord(usize, DP, DP),               // (n, Left/Right, Start/End)
    Sentence(usize, DP),                // (n, Left/Right)
    Para(usize, DP),                    // (n, Left/Right)
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
            Mto::Home(dp) => write!(f, "home({})", dp),
            Mto::End => write!(f, "end"),
            Mto::Row(n, dp) => write!(f, "row({},{})", n, dp),
            Mto::Percent(n) => write!(f, "percent({})", n),
            Mto::Cursor(n) => write!(f, "cursor({})", n),
            Mto::CharF(n, ch, dp) => write!(f, "charf({},{:?},{})", n, ch, dp),
            Mto::CharT(n, ch, dp) => write!(f, "chart({},{:?},{})", n, ch, dp),
            Mto::CharR(n, dp) => write!(f, "charr({},{})", n, dp),
            Mto::Word(n, dp1, dp2) => write!(f, "word({},{},{})", n, dp1, dp2),
            Mto::WWord(n, dp1, dp2) => write!(f, "wword({},{},{})", n, dp1, dp2),
            Mto::Sentence(n, dp) => write!(f, "sentence({},{})", n, dp),
            Mto::Para(n, dp) => write!(f, "para({},{})", n, dp),
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
    pub fn reverse(self, n: usize, dp: DP) -> Result<Self> {
        use Mto::{CharF, CharT, Pattern};

        let evnt = match (self, dp) {
            (CharF(_, ch, DP::Left), DP::Right) => CharF(n, ch, DP::Left),
            (CharF(_, ch, DP::Left), DP::Left) => CharF(n, ch, DP::Right),
            (CharF(_, ch, DP::Right), DP::Right) => CharF(n, ch, DP::Right),
            (CharF(_, ch, DP::Right), DP::Left) => CharF(n, ch, DP::Left),
            (CharT(_, ch, DP::Left), DP::Right) => CharT(n, ch, DP::Left),
            (CharT(_, ch, DP::Left), DP::Left) => CharT(n, ch, DP::Right),
            (CharT(_, ch, DP::Right), DP::Right) => CharT(n, ch, DP::Right),
            (CharT(_, ch, DP::Right), DP::Left) => CharT(n, ch, DP::Left),
            (Pattern(_, ch, DP::Left), DP::Right) => Pattern(n, ch, DP::Left),
            (Pattern(_, ch, DP::Left), DP::Left) => Pattern(n, ch, DP::Right),
            (Pattern(_, ch, DP::Right), DP::Right) => Pattern(n, ch, DP::Right),
            (Pattern(_, ch, DP::Right), DP::Left) => Pattern(n, ch, DP::Left),
            (Mto::None, _) => Mto::None,
            _ => err_at!(Fatal, msg: format!("unreachable"))?,
        };
        Ok(evnt)
    }
}

/// Event specific to application `code`.
#[derive(Clone, Eq, PartialEq)]
pub enum Code {
    Less(String),
    Prompt(String),
    StatusFile,
    StatusCursor,
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter) -> result::Result<(), fmt::Error> {
        use Code::{Less, Prompt, StatusCursor, StatusFile};

        match self {
            Less(_) => write!(f, "less"),
            Prompt(_) => write!(f, "prompt"),
            StatusFile => write!(f, "status_file"),
            StatusCursor => write!(f, "status_cursor"),
        }
    }
}

/// Event specific to application `code`.
#[derive(Clone, Eq, PartialEq)]
pub enum Ted {
    ShowConfig,
}

impl fmt::Display for Ted {
    fn fmt(&self, f: &mut fmt::Formatter) -> result::Result<(), fmt::Error> {
        use Ted::ShowConfig;

        match self {
            ShowConfig => write!(f, "show-config"),
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct Input {
    start_byte: usize,
    old_end_byte: usize,
    new_end_byte: usize,
    start_position: (usize, usize),   // (row, column) starts from ZERO
    old_end_position: (usize, usize), // (row, column) starts from ZERO
    new_end_position: (usize, usize), // (row, column) starts from ZERO
}

impl fmt::Display for Input {
    fn fmt(&self, f: &mut fmt::Formatter) -> result::Result<(), fmt::Error> {
        write!(
            f,
            "Input<{},({}->{})",
            self.start_byte, self.old_end_byte, self.new_end_byte
        )
    }
}

impl fmt::Debug for Input {
    fn fmt(&self, f: &mut fmt::Formatter) -> result::Result<(), fmt::Error> {
        write!(
            f,
            "Input<{},({}->{}):{:?},({:?}->{:?})",
            self.start_byte,
            self.old_end_byte,
            self.new_end_byte,
            self.start_position,
            self.old_end_position,
            self.new_end_position
        )
    }
}

impl Input {
    pub fn start_edit(
        start_byte: usize,
        old_end_byte: usize,
        start_position: (usize, usize),
        old_end_position: (usize, usize),
    ) -> Input {
        Input {
            start_byte,
            old_end_byte,
            new_end_byte: Default::default(),
            start_position,
            old_end_position,
            new_end_position: Default::default(),
        }
    }

    pub fn finish(mut self, new_eb: usize, new_ep: (usize, usize)) -> Event {
        self.new_end_byte = new_eb;
        self.new_end_position = new_ep;
        Event::Edit(self)
    }
}

impl From<Input> for ts::InputEdit {
    fn from(val: Input) -> Self {
        ts::InputEdit {
            start_byte: val.start_byte,
            old_end_byte: val.old_end_byte,
            new_end_byte: val.new_end_byte,
            start_position: ts::Point {
                row: val.start_position.0,
                column: val.start_position.1,
            },
            old_end_position: ts::Point {
                row: val.old_end_position.0,
                column: val.old_end_position.1,
            },
            new_end_position: ts::Point {
                row: val.new_end_position.0,
                column: val.new_end_position.1,
            },
        }
    }
}
