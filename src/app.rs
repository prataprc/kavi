//! Application specific traits and functions. For example `code` is a
//! ted-application.

use std::sync::mpsc;

use crate::{code, event::Event, pubsub::Notify, state, window::Cursor, Result};
#[allow(unused_imports)]
use crate::{pubsub::PubSub, window::Coord};

pub trait Application {
    /// Subscribe a channel for a topic. Any number of components can
    /// subscribe to the same topic. Refer [PubSub] for more detail.
    fn subscribe(&mut self, topic: &str, tx: mpsc::Sender<Notify>);

    /// Notify all subscribers for `topic` with `msg`. Refer [PubSub]
    /// for more detail.
    fn notify(&self, topic: &str, msg: Notify) -> Result<()>;

    /// Handle event. Refer [Event] for details.
    fn on_event(&mut self, evnt: Event) -> Result<Event>;

    /// Refresh terminal window, application is responsible for its view-port,
    /// typically configured using [Coord] when the application was
    /// created.
    fn on_refresh(&mut self) -> Result<()>;

    /// Return the cursor within application's view-port.
    fn to_cursor(&self) -> Option<Cursor>;

    /// Return a string less than the specified width.
    fn to_tab_title(&self, wth: usize) -> state::TabTitle;
}

pub enum App {
    Code(code::Code),
    None,
}

impl Default for App {
    fn default() -> App {
        App::None
    }
}

impl App {
    pub fn subscribe(&mut self, topic: &str, tx: mpsc::Sender<Notify>) {
        match self {
            App::Code(app) => app.subscribe(topic, tx),
            App::None => (),
        }
    }

    pub fn notify(&self, topic: &str, msg: Notify) -> Result<()> {
        match self {
            App::Code(app) => app.notify(topic, msg),
            App::None => Ok(()),
        }
    }

    pub fn to_cursor(&self) -> Option<Cursor> {
        match self {
            App::Code(app) => app.to_cursor(),
            App::None => None,
        }
    }

    pub fn on_event(&mut self, evnt: Event) -> Result<Event> {
        match self {
            App::Code(app) => app.on_event(evnt),
            App::None => Ok(evnt),
        }
    }

    pub fn on_refresh(&mut self) -> Result<()> {
        match self {
            App::Code(app) => app.on_refresh(),
            App::None => Ok(()),
        }
    }

    pub fn to_tab_title(&self, wth: usize) -> state::TabTitle {
        match self {
            App::Code(app) => app.to_tab_title(wth),
            App::None => unreachable!(),
        }
    }
}
