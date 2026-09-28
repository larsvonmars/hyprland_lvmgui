//! The island panel: what is playing, what is waiting, and what day it is.
//!
//! Three columns of information in one card that unfolds from the bar's clock:
//!
//! ```text
//!  ┌───────────── handle ─────────────┐
//!  │ 16:04                    [playing] [hidden]  │
//!  │ Friday, 25 September   ─────────────────────────  │
//!  │ ┌─ now playing ─────┐ ┌─ calendar ─────────────┐ │
//!  │ │ title             │ │ ‹ September 2026 ›     │ │
//!  │ │ artist            │ │ Mo Tu We Th Fr Sa Su   │ │
//!  │ │ ▁▁▁▁▁▁ 1:15 4:02  │ │ …                      │ │
//!  │ │ [‹] [▶] [›]       │ │                        │ │
//!  │ └───────────────────┘ └────────────────────────┘ │
//!  │ ┌─ notifications ──────────────────┐            │
//!  │ │ 3 unread notifications           │            │
//!  │ │ ▣ Firefox                     now│            │
//!  │ │   Test summary here              │            │
//!  │ │ ▣ Signal                   12 min│            │
//!  │ │   Another summary                │            │
//!  │ │ +1 more                          │            │
//!  │ │ [DND off] [Open]                 │            │
//!  │ │ [Hide] [Clear]                   │            │
//!  │ └──────────────────────────────────┘            │
//!  └──────────────────────────────────────────────────┘
//! ```
//!
//! The view paints what it is handed and runs the commands its buttons mean. It
//! does not read anything itself: `main` polls MPRIS while the panel is up, the
//! notification feed arrives on its own - both the state from `swaync` and the
//! rows off the bus (see `notify`) - and the clock ticks, so there is one place
//! that decides *when* to read, and one place that decides how to draw.
//!
//! One detail follows from the hover design: the panel's width is what its hot
//! zone is measured against, so a widget that wants more room than it was given
//! is a bug and not a layout surprise. Every label a notification can fill in is
//! therefore capped with [`text::cap_width`].
//!
//! The card itself - the fill, the border, the shadow, the 16px radius - is the
//! shell's (`box.card` in `base.css`), because this panel is a card like every
//! OSD in the collection; what is here is only what goes inside one.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use gtk::glib;
use gtk::prelude::*;

use hypr_osd_core::icons::{self, names};
use hypr_osd_core::mpris::Playback;
use hypr_osd_core::text;

use crate::calendar::Month;
use crate::notify::{self, History, Item, Notifications};
use crate::Settings;

/// How large the panel's own marks are drawn. Two sizes, because they answer
/// two questions: a heading's or a chip's icon sits beside a 10px word and is
/// part of it, while the transport and calendar buttons are what the pointer
/// aims at.
const SMALL_ICON: i32 = 12;
const BUTTON_ICON: i32 = 14;

/// The numbers a notification row is measured against. The first two are the
/// horizontal padding of `box.tile` and `box.notif-row` in `island.css`, and the
/// third is the icon badge plus the gap beside it: a row's text column is what is
/// left of the panel's left column once they are taken off.
const TILE_PAD_X: i32 = 10;
const ROW_PAD_X: i32 = 6;
const ROW_ART: i32 = 24 + 8;
/// The width of the age at the end of a row's first line - `59 min` is the
/// longest it ever gets.
const ROW_AGE: i32 = 38;

/// The weekday headings, Monday first - the same order [`Month::weeks`] lays the
/// grid out in.
const WEEKDAYS: [&str; 7] = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"];

pub struct IslandView {
    /// The panel's content: the card's child.
    pub root: gtk::Box,
    clock: gtk::Label,
    date: gtk::Label,
    chips: gtk::Box,
    track: gtk::Label,
    artist: gtk::Label,
    progress: gtk::ProgressBar,
    elapsed: gtk::Label,
    total: gtk::Label,
    play: gtk::Button,
    /// The row holding the progress bar and the times: hidden together, because a
    /// time without a bar says nothing.
    progress_row: gtk::Box,
    transport: gtk::Box,
    notif_count: gtk::Label,
    /// Where the notification rows are built.
    notif_rows: gtk::Box,
    /// The age label of every row, with the notification it belongs to: the one
    /// part of a row that has to change while nothing else does.
    ages: RefCell<Vec<(gtk::Label, Item)>>,
    /// How many rows the tile draws, and how wide the column they sit in is.
    rows: usize,
    column_width: i32,
    notif_actions: gtk::Box,
    dnd: gtk::Button,
    hide: gtk::Button,
    clear: gtk::Button,
    calendar_title: gtk::Label,
    calendar_grid: gtk::Grid,
    month: RefCell<Month>,
}

impl IslandView {
    pub fn new(settings: &Rc<Settings>) -> Rc<IslandView> {
        // ---- header: the clock, the date, and the state chips --------------
        let clock = gtk::Label::new(None);
        clock.add_css_class("island-clock");
        clock.set_xalign(0.0);
        let date = gtk::Label::new(None);
        date.add_css_class("island-date");
        date.set_xalign(0.0);
        let heading = gtk::Box::new(gtk::Orientation::Vertical, 1);
        heading.append(&clock);
        heading.append(&date);

        let chips = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        chips.add_css_class("chips");
        chips.set_valign(gtk::Align::Center);

        let header = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        header.add_css_class("island-header");
        header.append(&heading);
        header.append(&chips);

        // ---- media tile ----------------------------------------------------
        let track = gtk::Label::new(Some("Nothing playing"));
        track.add_css_class("track");
        track.set_xalign(0.0);
        track.set_ellipsize(gtk::pango::EllipsizeMode::End);
        track.set_max_width_chars(28);
        let artist = gtk::Label::new(None);
        artist.add_css_class("artist");
        artist.set_xalign(0.0);
        artist.set_ellipsize(gtk::pango::EllipsizeMode::End);
        artist.set_max_width_chars(30);

        let progress = gtk::ProgressBar::new();
        progress.set_show_text(false);
        progress.set_valign(gtk::Align::Center);
        let elapsed = gtk::Label::new(Some("0:00"));
        elapsed.add_css_class("time");
        elapsed.set_xalign(0.0);
        let total = gtk::Label::new(Some("0:00"));
        total.add_css_class("time");
        total.set_xalign(1.0);
        let times = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        times.append(&elapsed);
        times.append(&total);
        let progress_row = gtk::Box::new(gtk::Orientation::Vertical, 2);
        progress_row.append(&progress);
        progress_row.append(&times);

        let previous = transport_button(names::SKIP_BACK, "Previous track", || {
            let _ = hypr_osd_core::mpris::skip(hypr_osd_core::mpris::Direction::Previous);
        });
        let play = transport_button(names::PLAY, "Play or pause", || {
            let _ = hypr_osd_core::mpris::play_pause();
        });
        let next = transport_button(names::SKIP_FORWARD, "Next track", || {
            let _ = hypr_osd_core::mpris::skip(hypr_osd_core::mpris::Direction::Next);
        });
        let transport = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        transport.add_css_class("transport");
        transport.set_halign(gtk::Align::Center);
        for button in [&previous, &play, &next] {
            transport.append(button);
        }

        let media = tile("now playing", names::MUSIC);
        media.append(&track);
        media.append(&artist);
        media.append(&progress_row);
        media.append(&transport);

        // ---- notification tile ---------------------------------------------
        let notif_count = gtk::Label::new(Some("All caught up"));
        notif_count.add_css_class("notif-count");
        notif_count.set_xalign(0.0);

        // The rows. Nothing is put in here at build time: the first line of the
        // feed fills them in, and until then the count above says it all.
        let notif_rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
        notif_rows.add_css_class("notif-rows");

        let dnd = chip_button(names::BELL, "DND off", {
            let client = settings.notifications_command.clone();
            move || notify_command(&client, notify::Command::ToggleDnd)
        });
        let open = chip_button(names::LIST, "Open centre", {
            let client = settings.notifications_command.clone();
            move || notify_command(&client, notify::Command::ToggleCentre)
        });
        let hide = chip_button(names::EYE_OFF, "Hide all", {
            let client = settings.notifications_command.clone();
            move || notify_command(&client, notify::Command::HideShown)
        });
        let clear = chip_button(names::TRASH, "Clear all", {
            let client = settings.notifications_command.clone();
            move || notify_command(&client, notify::Command::DismissAll)
        });
        clear.add_css_class("danger");

        // Two rows of two: four buttons in a row would set the panel's width, and
        // the panel's width is the calendar's business.
        let notif_actions = gtk::Box::new(gtk::Orientation::Vertical, 6);
        notif_actions.add_css_class("notif-actions");
        for row in [[&dnd, &open], [&hide, &clear]] {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            for button in row {
                line.append(button);
            }
            notif_actions.append(&line);
        }

        let notifications = tile("notifications", names::BELL);
        notifications.append(&notif_count);
        notifications.append(&notif_rows);
        notifications.append(&notif_actions);

        let left = gtk::Box::new(gtk::Orientation::Vertical, 8);
        left.add_css_class("column");
        left.set_size_request(settings.left_width, -1);
        left.append(&media);
        left.append(&notifications);

        // ---- calendar tile -------------------------------------------------
        let calendar_title = gtk::Label::new(None);
        calendar_title.add_css_class("cal-title");
        let previous_month = icon_button(names::CHEVRON_LEFT, "Previous month");
        let next_month = icon_button(names::CHEVRON_RIGHT, "Next month");
        let title_row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        title_row.add_css_class("cal-head-row");
        title_row.append(&previous_month);
        title_row.append(&calendar_title);
        title_row.append(&next_month);

        let calendar_grid = gtk::Grid::new();
        calendar_grid.add_css_class("cal-grid");
        calendar_grid.set_row_spacing(2);
        calendar_grid.set_column_spacing(2);
        calendar_grid.set_halign(gtk::Align::Center);

        let calendar = tile("calendar", names::CLOCK);
        calendar.append(&title_row);
        calendar.append(&calendar_grid);

        let right = gtk::Box::new(gtk::Orientation::Vertical, 8);
        right.add_css_class("column");
        right.set_size_request(settings.right_width, -1);
        right.append(&calendar);

        // ---- the panel -----------------------------------------------------
        let handle = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        handle.add_css_class("handle");
        handle.set_halign(gtk::Align::Center);
        handle.set_size_request(46, 4);

        let body = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        body.add_css_class("body");
        body.append(&left);
        body.append(&right);

        let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
        separator.add_css_class("sep");

        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.add_css_class("island");
        root.append(&handle);
        root.append(&header);
        root.append(&separator);
        root.append(&body);

        let view = Rc::new(IslandView {
            root,
            clock,
            date,
            chips,
            track,
            artist,
            progress,
            elapsed,
            total,
            play,
            progress_row,
            transport,
            notif_count,
            notif_rows,
            ages: RefCell::new(Vec::new()),
            rows: settings.notification_rows,
            column_width: settings.left_width,
            notif_actions,
            dnd,
            hide,
            clear,
            calendar_title,
            calendar_grid,
            month: RefCell::new(Month::today()),
        });

        // The two calendar buttons are wired last, once the view exists: they
        // redraw the very grid they sit above.
        {
            let view = view.clone();
            previous_month.connect_clicked(move |_| view.shift_month(-1));
        }
        {
            let view = view.clone();
            next_month.connect_clicked(move |_| view.shift_month(1));
        }
        view.render_calendar();
        view
    }

    /// The clock and the date. Called once a second while the panel is up.
    pub fn render_clock(&self) {
        let now = glib::DateTime::now_local().ok();
        let text = |pattern: &str| {
            now.as_ref()
                .and_then(|now| now.format(pattern).ok())
                .map(|text| text.to_string())
                .unwrap_or_default()
        };
        self.clock.set_text(&text("%H:%M"));
        self.date.set_text(&text("%A, %d %B %Y"));
    }

    /// What is playing, or nothing at all.
    pub fn render_media(&self, playback: Option<&Playback>) {
        let Some(playback) = playback else {
            self.track.set_text("Nothing playing");
            self.artist.set_visible(false);
            self.progress_row.set_visible(false);
            self.transport.set_sensitive(false);
            set_button_icon(&self.play, names::PLAY);
            return;
        };

        self.track.set_text(&playback.track.title);
        let subtitle = playback.track.subtitle();
        self.artist.set_text(&subtitle);
        self.artist.set_visible(!subtitle.is_empty());

        // A stream with no length has no progress to draw; a bar stuck at zero
        // would claim it had just started.
        let known_length = playback.length > 0;
        self.progress_row.set_visible(known_length);
        if known_length {
            self.progress.set_fraction(playback.fraction());
            self.elapsed.set_text(&playback.elapsed());
            self.total.set_text(&playback.total());
        }

        self.transport.set_sensitive(true);
        set_button_icon(
            &self.play,
            if playback.playing {
                names::PAUSE
            } else {
                names::PLAY
            },
        );
    }

    /// The notification tile: the count, the rows, and the buttons that only make
    /// sense when there is something to act on.
    pub fn render_notifications(&self, notifications: &Notifications, history: &History) {
        let count = notifications.count;
        self.notif_count.set_text(&match count {
            0 => "All caught up".to_string(),
            1 => "1 unread notification".to_string(),
            count => format!("{count} unread notifications"),
        });
        // Nothing waiting is good news, and good news need not be the brightest
        // thing in the tile.
        set_class(&self.notif_count, "quiet", count == 0);

        label_of(&self.dnd).set_text(if notifications.dnd {
            "DND on"
        } else {
            "DND off"
        });
        set_chip_icon(
            &self.dnd,
            if notifications.dnd {
                names::BELL_OFF
            } else {
                names::BELL
            },
        );
        set_class(&self.dnd, "on", notifications.dnd);

        // Hiding or clearing nothing is not a thing to offer.
        let has_notifications = count > 0;
        self.hide.set_sensitive(has_notifications);
        self.clear.set_sensitive(has_notifications);
        self.notif_actions.set_visible(true);

        self.render_rows(history, notifications);
    }

    /// The rows: the newest few notifications the bus carried, made again for a
    /// new list.
    ///
    /// Made again rather than reconciled, because the list changes when a
    /// notification arrives - a handful of times an hour - and working out which
    /// row moved would be more code than building them over. The ages are the
    /// part that changes *without* a new list, and they are kept in `ages` for
    /// [`IslandView::render_ages`] to touch while the panel is up.
    fn render_rows(&self, history: &History, notifications: &Notifications) {
        while let Some(child) = self.notif_rows.first_child() {
            self.notif_rows.remove(&child);
        }
        self.ages.borrow_mut().clear();

        let now = Instant::now();
        let shown: Vec<&Item> = history.items().iter().take(self.rows).collect();
        for (position, item) in shown.iter().enumerate() {
            if position > 0 {
                // A hairline between two notifications rather than a box around
                // each: the tile is a surface already, and a box inside a box
                // inside a box is where a panel starts to look noisy.
                let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
                separator.add_css_class("notif-sep");
                self.notif_rows.append(&separator);
            }
            let (row, age) = notification_row(item, self.text_width(), now);
            self.notif_rows.append(&row);
            self.ages.borrow_mut().push((age, (*item).clone()));
        }

        // What the tile cannot show, admitted rather than hidden: the count is the
        // daemon's, and the rows are the ones that arrived while this process was
        // watching. With no row on screen at all - a panel that has just started,
        // or a bus that refuses to be monitored - there is nothing to count
        // against, so the line above stands alone.
        let hidden = usize::try_from(notifications.count)
            .unwrap_or(0)
            .saturating_sub(shown.len());
        if hidden > 0 && !shown.is_empty() {
            let more = gtk::Label::new(Some(&format!("+{hidden} more")));
            more.add_css_class("notif-more");
            more.set_xalign(0.0);
            self.notif_rows.append(&more);
        }
    }

    /// Keep the ages honest: a row that says `5 min` has to become `6 min` with
    /// nothing having arrived in between. Called on the heartbeat, and only while
    /// the panel is up.
    pub fn render_ages(&self) {
        let now = Instant::now();
        for (label, item) in self.ages.borrow().iter() {
            label.set_text(&item.age(now));
        }
    }

    /// How wide a row's text column is: the panel's left column, less the padding
    /// the tile and the row put around it, less the icon badge and the gap beside
    /// it.
    fn text_width(&self) -> i32 {
        (self.column_width - 2 * TILE_PAD_X - 2 * ROW_PAD_X - ROW_ART).max(40)
    }

    /// The state chips in the header: what is playing, what is waiting, and
    /// whether the control centre is up.
    pub fn render_chips(&self, playback: Option<&Playback>, notifications: &Notifications) {
        let mut wanted: Vec<(String, &str)> = Vec::new();
        if let Some(playback) = playback {
            wanted.push((
                if playback.playing {
                    "playing".to_string()
                } else {
                    "paused".to_string()
                },
                if playback.playing { "playing" } else { "dim" },
            ));
        }
        if notifications.count > 0 {
            wanted.push((format!("{} unread", notifications.count), "warn"));
        }
        if notifications.dnd {
            wanted.push(("DND".to_string(), "warn"));
        } else if notifications.inhibited {
            wanted.push(("inhibited".to_string(), "warn"));
        }
        if notifications.centre_open {
            wanted.push(("centre open".to_string(), "accent"));
        }

        // Rebuilding four labels on every repaint is cheaper than working out
        // which of them changed, and the repaints are once a second.
        while let Some(child) = self.chips.first_child() {
            self.chips.remove(&child);
        }
        for (text, class) in wanted {
            let chip = gtk::Label::new(Some(&text));
            chip.add_css_class("chip");
            chip.add_css_class(class);
            self.chips.append(&chip);
        }
    }

    /// Page the calendar by whole months.
    pub fn shift_month(&self, delta: i32) {
        let next = self.month.borrow().shift(delta);
        *self.month.borrow_mut() = next;
        self.render_calendar();
    }

    /// Draw the month grid.
    pub fn render_calendar(&self) {
        let month = *self.month.borrow();
        self.calendar_title.set_text(&month.title());

        while let Some(child) = self.calendar_grid.first_child() {
            self.calendar_grid.remove(&child);
        }

        let today = Month::today();
        let now = glib::DateTime::now_local().ok();
        let today_day = now.as_ref().map(|now| now.day_of_month()).unwrap_or(0);

        // The weekday headings, then the six weeks of the month.
        for (column, name) in WEEKDAYS.iter().enumerate() {
            let label = gtk::Label::new(Some(name));
            label.add_css_class("cal-head");
            self.calendar_grid.attach(&label, column as i32, 0, 1, 1);
        }
        for (row, week) in month.weeks().iter().enumerate() {
            for (column, day) in week.iter().enumerate() {
                let label = gtk::Label::new(None);
                label.add_css_class("cal-day");
                if *day > 0 {
                    label.set_text(&day.to_string());
                }
                // Saturday and Sunday sit at the right of a Monday-first week.
                if column >= 5 {
                    label.add_css_class("weekend");
                }
                if *day > 0 && *day == today_day && month == today {
                    label.add_css_class("today");
                }
                self.calendar_grid
                    .attach(&label, column as i32, row as i32 + 1, 1, 1);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Small building blocks
// ---------------------------------------------------------------------------

/// A tile: the nested surface inside the card, with its section heading.
fn tile(title: &str, icon: &str) -> gtk::Box {
    let heading = icons::lucide(icon, SMALL_ICON);
    heading.add_css_class("section-icon");
    let name = gtk::Label::new(Some(title));
    name.add_css_class("section");
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    line.add_css_class("section-row");
    line.append(&heading);
    line.append(&name);

    let tile = gtk::Box::new(gtk::Orientation::Vertical, 6);
    tile.add_css_class("tile");
    tile.append(&line);
    tile
}

/// One notification row: the application's badge, what it said, and how long ago.
///
/// The age label comes back with the row because it is the one part of a row that
/// changes while nothing else does - see [`IslandView::render_ages`].
fn notification_row(item: &Item, text_width: i32, now: Instant) -> (gtk::Box, gtk::Label) {
    // The badge the icon sits on. It is drawn whether or not there is a picture,
    // so every row has the same shape: the icon theme's answer decides what sits
    // inside it, not the box.
    const ART: i32 = 24;
    let art = gtk::Stack::new();
    art.add_css_class("notif-art");
    art.set_size_request(ART, ART);
    art.set_halign(gtk::Align::Start);
    art.set_valign(gtk::Align::Center);
    let image = gtk::Image::new();
    image.set_pixel_size(ART - 8);
    let letter = gtk::Label::new(Some(&text::initial(&item.app)));
    letter.add_css_class("notif-letter");
    art.add_named(&image, Some("icon"));
    art.add_named(&letter, Some("letter"));
    art.set_visible_child_name("letter");
    // The hint is an icon name or the path of a file the sender attached, and
    // `by_name` answers both. With no hint at all the application's own name is
    // the best guess - and that one goes through the desktop entries, because a
    // name is not an icon name until it has been looked up (`Telegram` is the
    // application, `telegram` the icon). An application the theme has nothing for
    // stands on its first letter, which still says which one it is.
    let paintable = if item.icon.is_empty() {
        icons::paintable(&item.app, ART - 8)
    } else {
        icons::by_name(&item.icon, ART - 8)
    };
    if let Some(paintable) = paintable {
        image.set_paintable(Some(&paintable));
        art.set_visible_child_name("icon");
    }

    // The application and the age on one line, with the age pushed to the end.
    // A sender that names no application gets its summary up there instead - and
    // then there is nothing left for a second line.
    let named = !item.app.is_empty();
    let app = gtk::Label::new(Some(if named { &item.app } else { &item.title }));
    app.add_css_class("notif-app");
    app.set_xalign(0.0);
    app.set_ellipsize(gtk::pango::EllipsizeMode::End);
    app.set_hexpand(true);
    let age = gtk::Label::new(Some(&item.age(now)));
    age.add_css_class("notif-age");
    age.set_xalign(1.0);
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    head.append(&app);
    head.append(&age);

    let title = gtk::Label::new(Some(&item.title));
    title.add_css_class("notif-text");
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);

    let words = gtk::Box::new(gtk::Orientation::Vertical, 1);
    words.set_hexpand(true);
    words.append(&head);
    if named {
        words.append(&title);
    }

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.add_css_class("notif-row");
    if item.urgency >= 2 {
        // What swaync holds on screen until it is dealt with, so it is worth
        // noticing at a glance in the list as well.
        row.add_css_class("critical");
    }
    // What the sender wrote, in full: the row has room for one line of it.
    row.set_tooltip_text(Some(&item.tooltip()));
    row.append(&art);
    row.append(&words);

    // Capped, both of them: an ellipsised label still asks for its whole text when
    // it is measured, and a row that asked for too much would widen the column -
    // and with it the card, whose width is what the panel's hover zone is
    // measured against.
    text::cap_width(&app, text_width - ROW_AGE - 6);
    text::cap_width(&title, text_width);

    (row, age)
}

/// A button with an icon and a word, the way every control in this panel is
/// built: the icon carries the meaning, the word removes the doubt.
fn chip_button(icon: &str, label: &str, action: impl Fn() + 'static) -> gtk::Button {
    let icon = icons::lucide(icon, SMALL_ICON);
    let text = gtk::Label::new(Some(label));
    text.add_css_class("chip-text");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.append(&icon);
    row.append(&text);

    let button = gtk::Button::new();
    button.add_css_class("chipbtn");
    button.set_child(Some(&row));
    button.set_focus_on_click(false);
    button.set_can_focus(false);
    button.connect_clicked(move |_| action());
    button
}

/// A button that is only an icon, for the transport and the calendar paging.
fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::new();
    button.set_child(Some(&icons::lucide(icon, BUTTON_ICON)));
    button.add_css_class("flat");
    button.set_tooltip_text(Some(tooltip));
    button.set_focus_on_click(false);
    button.set_can_focus(false);
    button
}

fn transport_button(icon: &str, tooltip: &str, action: impl Fn() + 'static) -> gtk::Button {
    let button = icon_button(icon, tooltip);
    button.add_css_class("transport-button");
    button.connect_clicked(move |_| action());
    button
}

/// The text label inside a [`chip_button`] - the half that changes wording while
/// the icon stays.
fn label_of(button: &gtk::Button) -> gtk::Label {
    button
        .child()
        .and_downcast::<gtk::Box>()
        .and_then(|row| row.last_child())
        .and_downcast::<gtk::Label>()
        .expect("a chip button's second child is its label")
}

/// The icon inside a [`chip_button`] - the half that stays while the word
/// beside it changes.
fn icon_of(button: &gtk::Button) -> gtk::Image {
    button
        .child()
        .and_downcast::<gtk::Box>()
        .and_then(|row| row.first_child())
        .and_downcast::<gtk::Image>()
        .expect("a chip button's first child is its icon")
}

/// Re-point a chip's icon at another drawing, which is how the DND switch says
/// which way it is switched without the button being rebuilt.
fn set_chip_icon(button: &gtk::Button, icon: &str) {
    icons::set_lucide(&icon_of(button), icon, SMALL_ICON);
}

/// The same for a button that is *only* an icon (see [`icon_button`]): the play
/// button becomes a pause in place, because rebuilding it would take the button
/// out from under the pointer that is resting on it.
fn set_button_icon(button: &gtk::Button, icon: &str) {
    if let Some(image) = button.child().and_downcast::<gtk::Image>() {
        icons::set_lucide(&image, icon, BUTTON_ICON);
    }
}

fn set_class(widget: &impl IsA<gtk::Widget>, class: &str, on: bool) {
    if on {
        widget.add_css_class(class);
    } else {
        widget.remove_css_class(class);
    }
}

/// Run one of the daemon's commands. Fire and forget: what changes comes back
/// through the notification feed, so the panel never guesses at the result.
fn notify_command(program: &str, command: notify::Command) {
    let _ = std::process::Command::new(program)
        .args(command.args())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}
