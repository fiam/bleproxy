//! Streams the proxy's logs to API clients that subscribe to them, as
//! ESPHome devices do. Home Assistant shows them in its own log when the
//! ESPHome entry's "subscribe to logs" option is on.
//!
//! [`Logs::layer`] formats the proxy's own events as ESPHome log lines
//! (`[I][server]: connection{peer=…}: client connected`) and publishes
//! them; each connection holds a [`LogSubscriber`] that receives them.
//! Nothing is formatted while no client wants the event's level.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use bleproxy_protocol::log_level;
use tokio::sync::broadcast;
use tracing::subscriber::Interest;
use tracing::{Level, Metadata, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::{Context, Filter};
use tracing_subscriber::registry::LookupSpan;

/// Lines kept for subscribers that fall behind.
const BACKLOG: usize = 256;

/// Target prefix of the proxy's own events; other crates' events, such as
/// D-Bus traffic, stay out of clients' logs.
const TARGET: &str = "bleproxy";

/// A formatted log line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    /// Its [`log_level`].
    pub level: u32,
    /// The line, as ESPHome formats it.
    pub text: String,
}

/// Where log lines are published, shared by the tracing layer and every
/// connection.
#[derive(Debug)]
pub struct Logs {
    lines: broadcast::Sender<LogLine>,
    /// Subscribers by the level they want, indexed by [`log_level`].
    subscribers: [AtomicUsize; 8],
}

impl Default for Logs {
    fn default() -> Self {
        Self {
            lines: broadcast::channel(BACKLOG).0,
            subscribers: Default::default(),
        }
    }
}

impl Logs {
    /// Subscribes to lines up to `level`, one of the [`log_level`] values,
    /// until the returned subscriber is dropped.
    #[must_use]
    pub fn subscribe(self: &Arc<Self>, level: u32) -> LogSubscriber {
        let level = level.min(log_level::VERY_VERBOSE);
        self.count(level).fetch_add(1, Ordering::Relaxed);
        LogSubscriber {
            logs: Arc::clone(self),
            level,
            lines: self.lines.subscribe(),
        }
    }

    /// Returns a tracing layer that publishes the proxy's events to
    /// subscribers.
    pub fn layer<S>(self: &Arc<Self>) -> impl Layer<S> + use<S>
    where
        S: Subscriber + for<'a> LookupSpan<'a>,
    {
        tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .without_time()
            .with_level(false)
            .with_target(false)
            .with_writer(Writer(Arc::clone(self)))
            .with_filter(Wanted(Arc::clone(self)))
    }

    /// Returns the most detailed level any subscriber wants.
    fn wanted(&self) -> u32 {
        (log_level::ERROR..=log_level::VERY_VERBOSE)
            .rev()
            .find(|&level| self.count(level).load(Ordering::Relaxed) > 0)
            .unwrap_or(log_level::NONE)
    }

    fn count(&self, level: u32) -> &AtomicUsize {
        let index = usize::try_from(level).unwrap_or(usize::MAX);
        // Levels are clamped on subscription; the last slot is the most
        // detailed level.
        let last = self.subscribers.len() - 1;
        &self.subscribers[index.min(last)]
    }

    fn publish(&self, line: LogLine) {
        // No subscribers is fine: nobody wants the line.
        let _ = self.lines.send(line);
    }
}

/// Lets through the proxy's events that some subscriber wants.
struct Wanted(Arc<Logs>);

impl<S> Filter<S> for Wanted {
    fn enabled(&self, metadata: &Metadata<'_>, _: &Context<'_, S>) -> bool {
        esphome_level(*metadata.level()) <= self.0.wanted()
    }

    fn callsite_enabled(&self, metadata: &'static Metadata<'static>) -> Interest {
        if metadata.target().starts_with(TARGET) {
            // Subscriptions come and go, so ask about every event.
            Interest::sometimes()
        } else {
            Interest::never()
        }
    }

    fn max_level_hint(&self) -> Option<LevelFilter> {
        Some(LevelFilter::TRACE)
    }
}

/// A client's subscription to log lines up to a level.
#[derive(Debug)]
pub struct LogSubscriber {
    logs: Arc<Logs>,
    level: u32,
    lines: broadcast::Receiver<LogLine>,
}

impl LogSubscriber {
    /// Waits for the next line at or below the subscribed level.
    ///
    /// # Errors
    ///
    /// Returns [`broadcast::error::RecvError::Lagged`] with the number of
    /// lines skipped when the subscriber fell behind.
    pub async fn recv(&mut self) -> Result<LogLine, broadcast::error::RecvError> {
        loop {
            let line = self.lines.recv().await?;
            if line.level <= self.level {
                return Ok(line);
            }
        }
    }
}

impl Drop for LogSubscriber {
    fn drop(&mut self) {
        self.logs.count(self.level).fetch_sub(1, Ordering::Relaxed);
    }
}

/// Maps a tracing level to ESPHome's: trace events are verbose.
const fn esphome_level(level: Level) -> u32 {
    match level {
        Level::ERROR => log_level::ERROR,
        Level::WARN => log_level::WARN,
        Level::INFO => log_level::INFO,
        Level::DEBUG => log_level::DEBUG,
        Level::TRACE => log_level::VERBOSE,
    }
}

/// The letter ESPHome starts a line of `level` with.
const fn level_letter(level: u32) -> &'static str {
    match level {
        log_level::ERROR => "E",
        log_level::WARN => "W",
        log_level::INFO => "I",
        log_level::CONFIG => "C",
        log_level::DEBUG => "D",
        log_level::VERBOSE => "V",
        _ => "VV",
    }
}

/// Formats `message` as an ESPHome log line of `level` with `tag`.
#[must_use]
pub fn format_line(level: u32, tag: &str, message: &str) -> String {
    format!("[{}][{tag}]: {message}", level_letter(level))
}

/// Tags lines with the module that logged them, such as `server` for
/// `bleproxy::server` and `bluez` for `bleproxy::bluez::gatt`.
fn tag(target: &str) -> &str {
    target
        .strip_prefix(TARGET)
        .and_then(|rest| rest.strip_prefix("::"))
        .and_then(|rest| rest.split("::").next())
        .filter(|module| !module.is_empty())
        .unwrap_or(TARGET)
}

/// Gives the fmt layer a buffer per event.
struct Writer(Arc<Logs>);

impl<'a> MakeWriter<'a> for Writer {
    type Writer = EventWriter<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        EventWriter {
            logs: &self.0,
            level: log_level::INFO,
            tag: TARGET.to_owned(),
            buffer: Vec::new(),
        }
    }

    fn make_writer_for(&'a self, metadata: &Metadata<'_>) -> Self::Writer {
        EventWriter {
            logs: &self.0,
            level: esphome_level(*metadata.level()),
            tag: tag(metadata.target()).to_owned(),
            buffer: Vec::new(),
        }
    }
}

/// Collects one formatted event and publishes it when dropped.
struct EventWriter<'a> {
    logs: &'a Logs,
    level: u32,
    tag: String,
    buffer: Vec<u8>,
}

impl io::Write for EventWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.buffer.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for EventWriter<'_> {
    fn drop(&mut self) {
        let message = String::from_utf8_lossy(&self.buffer);
        let message = message.trim_end();
        if message.is_empty() {
            return;
        }
        self.logs.publish(LogLine {
            level: self.level,
            text: format_line(self.level, &self.tag, message),
        });
    }
}

#[cfg(test)]
mod tests {
    use tracing_subscriber::layer::SubscriberExt as _;

    use super::*;

    /// Runs `f` with only `logs`' layer installed.
    fn with_layer(logs: &Arc<Logs>, f: impl FnOnce()) {
        let subscriber = tracing_subscriber::registry().with(logs.layer());
        tracing::subscriber::with_default(subscriber, f);
    }

    #[test]
    fn formats_lines_like_esphome() {
        assert_eq!(
            format_line(log_level::INFO, "server", "client connected"),
            "[I][server]: client connected"
        );
        assert_eq!(
            format_line(log_level::CONFIG, "bleproxy", "Name: pichu"),
            "[C][bleproxy]: Name: pichu"
        );
        assert_eq!(esphome_level(Level::TRACE), log_level::VERBOSE);
        assert_eq!(level_letter(log_level::VERY_VERBOSE), "VV");
    }

    #[test]
    fn tags_lines_with_modules() {
        assert_eq!(tag("bleproxy::server"), "server");
        assert_eq!(tag("bleproxy::bluez::gatt"), "bluez");
        assert_eq!(tag("bleproxy"), "bleproxy");
        assert_eq!(tag("bleproxy::"), "bleproxy");
    }

    #[test]
    fn follows_the_most_detailed_subscription() {
        let logs = Arc::new(Logs::default());
        assert_eq!(logs.wanted(), log_level::NONE);
        let info = logs.subscribe(log_level::INFO);
        let debug = logs.subscribe(log_level::DEBUG);
        assert_eq!(logs.wanted(), log_level::DEBUG);
        drop(debug);
        assert_eq!(logs.wanted(), log_level::INFO);
        // Unknown levels count as the most detailed one.
        let everything = logs.subscribe(99);
        assert_eq!(logs.wanted(), log_level::VERY_VERBOSE);
        drop((info, everything));
        assert_eq!(logs.wanted(), log_level::NONE);
    }

    #[tokio::test]
    async fn publishes_wanted_events_of_the_proxy() {
        let logs = Arc::new(Logs::default());
        let mut info = logs.subscribe(log_level::INFO);
        let mut debug = logs.subscribe(log_level::DEBUG);
        with_layer(&logs, || {
            tracing::info!(target: "bleproxy::server", peer = "192.168.1.104", "client connected");
            tracing::debug!(target: "bleproxy::server", "forwarded advertisements");
            tracing::trace!(target: "bleproxy::server", "too detailed");
            tracing::info!(target: "zbus", "not ours");
            tracing::warn!(target: "bleproxy", "done");
        });
        let connected = LogLine {
            level: log_level::INFO,
            text: "[I][server]: client connected peer=\"192.168.1.104\"".to_owned(),
        };
        assert_eq!(info.recv().await.unwrap(), connected);
        assert_eq!(debug.recv().await.unwrap(), connected);
        assert_eq!(
            debug.recv().await.unwrap().text,
            "[D][server]: forwarded advertisements"
        );
        // The info subscriber skips the debug line.
        let done = "[W][bleproxy]: done";
        assert_eq!(info.recv().await.unwrap().text, done);
        assert_eq!(debug.recv().await.unwrap().text, done);
    }
    #[tokio::test]
    async fn formats_nothing_without_subscribers() {
        let logs = Arc::new(Logs::default());
        let mut lines = logs.lines.subscribe();
        let log = || tracing::error!(target: "bleproxy", "maybe heard");
        with_layer(&logs, log);
        assert!(lines.try_recv().is_err());
        // The same event is published once someone subscribes.
        let mut subscriber = logs.subscribe(log_level::ERROR);
        with_layer(&logs, log);
        assert_eq!(
            subscriber.recv().await.unwrap().text,
            "[E][bleproxy]: maybe heard"
        );
    }
}
