//! What the viewer says about what happened, for the status bar and its
//! log. A message is one of three kinds: news of what was just done (shown
//! for a few seconds), work under way (shown while it runs) or a warning
//! (an error or something to know, shown until dismissed or followed by
//! another message). The last [`KEPT`] stay in the log, whichever.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// What kind of message it is, which says how long it is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    /// What was just done: shown for [`SHOWN_FOR`].
    Info,
    /// Work under way: shown while the application is busy.
    Progress,
    /// An error or a warning: shown until dismissed or followed.
    Warning,
}

#[derive(Clone, Debug)]
pub struct Message {
    pub text: String,
    pub level: Level,
    pub at: Instant,
}

/// How long news stays in the status bar.
pub const SHOWN_FOR: Duration = Duration::from_secs(6);
/// How many messages the log keeps.
pub const KEPT: usize = 50;

/// The messages of a document, the newest last.
#[derive(Clone, Debug, Default)]
pub struct Messages {
    log: VecDeque<Message>,
    dismissed: bool,
}

impl Messages {
    /// News of what was just done.
    pub fn info(&mut self, text: impl Into<String>) {
        self.push(text.into(), Level::Info);
    }

    /// Work started, said until it ends (or something else is said).
    pub fn progress(&mut self, text: impl Into<String>) {
        self.push(text.into(), Level::Progress);
    }

    /// An error or a warning, said until dismissed.
    pub fn warn(&mut self, text: impl Into<String>) {
        self.push(text.into(), Level::Warning);
    }

    /// `r`'s message: news when it is `Ok`, a warning when not.
    pub fn result(&mut self, r: Result<String, String>) {
        match r {
            Ok(m) => self.info(m),
            Err(e) => self.warn(e),
        }
    }

    pub fn push(&mut self, text: String, level: Level) {
        if text.is_empty() {
            return;
        }
        if self.log.len() == KEPT {
            self.log.pop_front();
        }
        self.log.push_back(Message {
            text,
            level,
            at: Instant::now(),
        });
        self.dismissed = false;
    }

    /// The newest message, shown or not.
    pub fn last(&self) -> Option<&Message> {
        self.log.back()
    }

    /// The newest message's text.
    pub fn last_text(&self) -> Option<&str> {
        self.last().map(|m| m.text.as_str())
    }

    /// The message the status bar shows at `now`: the newest, while its
    /// kind says (work under way while `busy`).
    pub fn current(&self, now: Instant, busy: bool) -> Option<&Message> {
        let m = self.log.back().filter(|_| !self.dismissed)?;
        let shown = match m.level {
            Level::Info => now.saturating_duration_since(m.at) < SHOWN_FOR,
            Level::Progress => busy,
            Level::Warning => true,
        };
        shown.then_some(m)
    }

    /// How long until the news shown at `now` goes (to draw the bar again
    /// then).
    pub fn expires_in(&self, now: Instant) -> Option<Duration> {
        let m = self.log.back().filter(|_| !self.dismissed)?;
        (m.level == Level::Info)
            .then(|| SHOWN_FOR.checked_sub(now.saturating_duration_since(m.at)))
            .flatten()
    }

    /// Stop showing the newest message (it stays in the log).
    pub fn dismiss(&mut self) {
        self.dismissed = true;
    }

    /// Every message kept, the oldest first.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &Message> {
        self.log.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.log.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn news_goes_warnings_stay_work_while_busy() {
        let mut m = Messages::default();
        let now = Instant::now();
        assert!(m.current(now, false).is_none());

        m.info("loaded 3 cells in 0.1 s");
        assert!(m.current(now, false).is_some());
        assert!(m.expires_in(now).is_some());
        let later = now + SHOWN_FOR + Duration::from_secs(1);
        assert!(m.current(later, false).is_none());
        assert_eq!(m.last_text(), Some("loaded 3 cells in 0.1 s"));

        m.progress("reloading…");
        assert!(m.current(later, true).is_some());
        assert!(m.current(later, false).is_none());

        m.warn("could not reload");
        let much_later = later + Duration::from_secs(3600);
        assert!(m.current(much_later, false).is_some());
        assert!(m.expires_in(much_later).is_none());
        m.dismiss();
        assert!(m.current(much_later, false).is_none());
        assert_eq!(m.iter().count(), 3);

        // A new message is shown again.
        m.info("saved");
        assert_eq!(m.current(Instant::now(), false).unwrap().text, "saved");
    }

    #[test]
    fn the_log_keeps_the_newest() {
        let mut m = Messages::default();
        for k in 0..KEPT + 5 {
            m.info(format!("{k}"));
        }
        assert_eq!(m.iter().count(), KEPT);
        assert_eq!(m.iter().next().unwrap().text, "5");
        m.info("");
        assert_eq!(m.last_text(), Some(&*format!("{}", KEPT + 4)));
    }
}
