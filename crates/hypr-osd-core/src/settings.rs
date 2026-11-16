//! The catalogue the settings window is built from.
//!
//! Every element reads its own `~/.config/hypr-osd/<element>.conf` and carries a
//! default in its source (see [`crate::config`]). That is the *whole* surface of
//! what can be tuned, and it is not something a window can discover: a `.conf`
//! file says what the user has already set, never what could be set, and the
//! defaults live in twelve different binaries.
//!
//! So this module is the missing half: one table per element, naming the keys a
//! person actually wants to reach, with the label, the sentence of help, the
//! built-in default and the shape of the control that edits it. The settings
//! window draws from it, and *only* from it - the file stays the source of truth
//! for what is set, and any key the file sets that this table does not know
//! about is still shown (as plain text), so the catalogue can be incomplete
//! without the window being wrong about a machine.
//!
//! Two rules keep it honest, and a test holds both:
//!
//! * a default here has to be the element's real default (they are copied from
//!   the `const DEFAULT_*` block of each `main.rs`), because the window shows it
//!   beside an unset key; and
//! * a default has to be *valid* for its own kind, so the control cannot be
//!   built around a value it cannot represent.

use crate::icons::names;
use crate::keymap::Keymap;

/// What kind of value a setting is, and therefore what edits it.
///
/// The window has one control per kind and nothing else: a switch, a spinner, a
/// fraction, a line of text, or a dropdown. There is deliberately no free-form
/// "type a number in" case beyond those - the numbers here all have a sane
/// range, and a spinner is a slider that can also be typed into.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    /// A switch. The file's own spelling - `true`/`false` - is what is written.
    Bool,
    /// A whole number: pixels, counts, milliseconds. `min`/`max` bound the
    /// spinner and `step` is how far one click moves it.
    Int { min: i32, max: i32, step: i32 },
    /// A fraction between two values, written as a decimal - the cards' share of
    /// the screen's height, mostly.
    Fraction { min: f64, max: f64 },
    /// A line of text: a command, an interface name, a date format.
    Text,
    /// A path to a file on this machine - a picture, mostly. Drawn as a text field
    /// with a folder button that opens GTK's file chooser, because a path is the
    /// one setting nobody should have to type correctly.
    Path,
    /// A path that may be a **file or a directory** - hyprpaper's wallpaper, which
    /// turns into a slideshow when it names a directory. Drawn like [`Kind::Path`],
    /// with a second button that picks a folder.
    PathFolder,
    /// A colour, written the way every file in this collection spells one:
    /// `#rrggbb` (or `#rrggbbaa`). Drawn as a colour swatch that opens GTK's
    /// colour chooser.
    Color,
    /// A font **family** name - what Pango and hyprlock take. Drawn as a button
    /// that opens GTK's font chooser; the size stays a separate row, because the
    /// screens here size their type independently of the family.
    Font,
    /// One of a handful of words this table can spell out itself (`adaptive`,
    /// `flat`): a dropdown, which is the only way to get one of them *right*.
    Choice(&'static [&'static str]),
    /// One of the names *this machine* knows, which no table can hold: the
    /// keyboard layouts, their variants, the models and the options. The window
    /// asks [`crate::keymap`] for the list when it draws the page, and falls back
    /// to a text field on a machine that has no such data.
    Keymap(Keymap),
    /// One of the names *one output* knows: the modes the monitor reports, the
    /// scales worth offering, the outputs it can mirror. Like [`Kind::Keymap`]
    /// the list is resolved while the page is drawn and the row falls back to a
    /// text field when there is none - but this list belongs to a display, so it
    /// comes from the compositor (`hyprctl`) rather than from a file.
    Monitor(MonitorList),
    /// A **reading**, not a setting: what the machine is doing right now. Drawn
    /// as a row with no control at all, because there is nothing to change - it
    /// is how the System tab opens with the data its options are about. Its
    /// `value` is made up while the page is drawn (see [`crate::energy`]), so it
    /// is never a key in a config file.
    Info,
    /// The buttons a page offers, which run a verb of the element the page
    /// belongs to. The Energy page's two actions - "save now" and "put the
    /// normal values back" - are not settings and cannot be spelled as one, so
    /// they get their own kind rather than being squeezed into a value.
    Actions(&'static [Command]),
}

/// One button on a page: the id the element turns into a verb, and how it is
/// drawn.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Command {
    /// What the page reports when the button is pressed; the element maps it to
    /// a verb (`apply`, `restore`).
    pub id: &'static str,
    pub label: &'static str,
    /// The Lucide drawing on the button.
    pub icon: &'static str,
}

/// Which list a display row picks from - the fields whose values are names this
/// machine (and this monitor) has, rather than free text.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MonitorList {
    /// The monitor's own modes (`2560x1440@59.95`), plus `preferred`.
    Mode,
    /// The scales worth offering (`1`, `1.25`, `1.5`, …), plus `auto`.
    Scale,
    /// The other outputs, plus `none`.
    Mirror,
    /// The connected outputs, plus [`PRIMARY_FOCUSED`]. The one list that is *not*
    /// about the row's own monitor: the primary display is a property of the
    /// collection rather than of an output (see [`PRIMARY_DISPLAY_SETTING`]).
    Primary,
}

/// One tunable key of one element.
pub struct Setting {
    /// The key, exactly as the `.conf` file spells it.
    pub key: &'static str,
    pub label: &'static str,
    /// One sentence: what changing it does. Shown under the label.
    pub help: &'static str,
    /// The element's built-in default, spelled the way the file writes it.
    pub default: &'static str,
    pub kind: Kind,
}

// --- the little constructors, so the table below reads like a table ---------

const fn flag(
    key: &'static str,
    label: &'static str,
    help: &'static str,
    default: bool,
) -> Setting {
    Setting {
        key,
        label,
        help,
        default: if default { "true" } else { "false" },
        kind: Kind::Bool,
    }
}

const fn number(
    key: &'static str,
    label: &'static str,
    help: &'static str,
    default: &'static str,
    min: i32,
    max: i32,
) -> Setting {
    Setting {
        key,
        label,
        help,
        default,
        kind: Kind::Int { min, max, step: 1 },
    }
}

/// A millisecond setting: an integer, but with a `step` that makes one click
/// worth a tenth of a second rather than one millisecond.
const fn millis(
    key: &'static str,
    label: &'static str,
    help: &'static str,
    default: &'static str,
    min: i32,
    max: i32,
) -> Setting {
    Setting {
        key,
        label,
        help,
        default,
        kind: Kind::Int {
            min,
            max,
            step: 100,
        },
    }
}

const fn fraction(
    key: &'static str,
    label: &'static str,
    help: &'static str,
    default: &'static str,
    min: f64,
    max: f64,
) -> Setting {
    Setting {
        key,
        label,
        help,
        default,
        kind: Kind::Fraction { min, max },
    }
}

const fn text(
    key: &'static str,
    label: &'static str,
    help: &'static str,
    default: &'static str,
) -> Setting {
    Setting {
        key,
        label,
        help,
        default,
        kind: Kind::Text,
    }
}

/// A path on this machine: a text field with a file chooser beside it.
const fn path(
    key: &'static str,
    label: &'static str,
    help: &'static str,
    default: &'static str,
) -> Setting {
    Setting {
        key,
        label,
        help,
        default,
        kind: Kind::Path,
    }
}

/// A path that may also be a directory: the same field, with a folder button too.
const fn path_folder(
    key: &'static str,
    label: &'static str,
    help: &'static str,
    default: &'static str,
) -> Setting {
    Setting {
        key,
        label,
        help,
        default,
        kind: Kind::PathFolder,
    }
}

/// A colour: a field whose value is `#rrggbb`, edited with GTK's colour chooser.
const fn color(
    key: &'static str,
    label: &'static str,
    help: &'static str,
    default: &'static str,
) -> Setting {
    Setting {
        key,
        label,
        help,
        default,
        kind: Kind::Color,
    }
}

/// A font family, picked from the fonts the machine has.
const fn font(
    key: &'static str,
    label: &'static str,
    help: &'static str,
    default: &'static str,
) -> Setting {
    Setting {
        key,
        label,
        help,
        default,
        kind: Kind::Font,
    }
}

/// A value picked from a list rather than typed - the list being one this table
/// can spell out.
const fn choice(
    key: &'static str,
    label: &'static str,
    help: &'static str,
    default: &'static str,
    options: &'static [&'static str],
) -> Setting {
    Setting {
        key,
        label,
        help,
        default,
        kind: Kind::Choice(options),
    }
}

/// A value picked from the names the machine's own XKB data has (see
/// [`crate::keymap`]).
const fn xkb(
    key: &'static str,
    label: &'static str,
    help: &'static str,
    default: &'static str,
    list: Keymap,
) -> Setting {
    Setting {
        key,
        label,
        help,
        default,
        kind: Kind::Keymap(list),
    }
}

/// A display row that picks from a list the *machine* has (see [`MonitorList`]):
/// the modes a monitor reports, or the scales worth offering.
const fn display(
    key: &'static str,
    label: &'static str,
    help: &'static str,
    default: &'static str,
    list: MonitorList,
) -> Setting {
    Setting {
        key,
        label,
        help,
        default,
        kind: Kind::Monitor(list),
    }
}

/// Who a page's values are *for* - which decides what making them take effect
/// means.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Applies {
    /// An element of this collection: its own `<file>.conf`, read once at
    /// start-up, so the page's chip restarts that element.
    Element,
    /// Hyprland itself: the options are written out as Lua and handed to the
    /// compositor (see [`crate::desktop`]), which applies them at once - the
    /// page's chip re-applies the file instead of restarting anything.
    Compositor,
    /// The *connected outputs*: Hyprland's monitor rules, which are not options
    /// but `hl.monitor({...})` calls (see [`crate::desktop`]). The catalogue names
    /// the fields of one rule ([`MONITOR_SETTINGS`]); the window makes one row per
    /// field per output when it draws the page, because which outputs exist is
    /// the compositor's to say. A change is applied like a Hyprland one, at once.
    Displays,
    /// The **System** tab's energy policy: an element's own file
    /// (`~/.config/hypr-osd/energy.conf`), read once at start-up by the
    /// `hypr-osd-energy` daemon - so a change takes effect when that daemon is
    /// restarted, exactly like any element's page. What makes it its own case is
    /// the *readings* the page opens with and the two buttons it carries, neither
    /// of which is a key in the file (see [`Kind::Info`] and [`Kind::Actions`]).
    Energy,
    /// The **Lock screen** page: hyprlock's own look. Its keys live in
    /// `~/.config/hypr-osd/lock.conf` like an element's, and what they change is
    /// the *generated* half of `~/.config/hypr/hyprlock.conf` - the variables it
    /// is written in terms of, palette included (see [`crate::lock`]).
    ///
    /// What makes it its own case is that there is nothing to restart and nothing
    /// to re-apply in the compositor: hyprlock reads its config at every lock, so a
    /// change is live the moment the variables are rewritten. The chip is there
    /// for a `theme.css` that was edited by hand - it rewrites the same file the
    /// change does.
    Lock,
    /// The **Touch** page: the touchscreen side of the collection - whether a
    /// finger is treated as a finger at all, which on-screen keyboard a tablet
    /// gets, and how long a long press lasts.
    ///
    /// It is not an element's page even though it has a file of its own
    /// ([`crate::touch::ELEMENT`]): the questions it answers are about the
    /// *machine*, and every element reads the same `touch.conf`, so there is no
    /// one binary for a restart chip to stop and start. A change takes effect
    /// when the elements that read it are restarted (`scripts/install.sh` does
    /// that for the ones that are running), which is what the page's blurb says.
    Touch,
    /// The **Shortcuts** page: the keys this collection implements, list *only*.
    /// It has no config file and no values to write - what it shows is
    /// [`SHORTCUTS`] - so its rows are readings with the keys in place of a name,
    /// and its chip says nothing (there is nothing to restart or re-apply).
    ///
    /// The source of truth for the bound half is `scripts/hyprland/osd.lua`, the
    /// template the installer writes into `~/.config/hypr/osd.lua`, and a test
    /// holds this table against that file: a key bound there and missing here is a
    /// test failure, which is what makes the page's claim to be complete honest.
    /// Reading the *running* compositor cannot do that job: Hyprland 0.56 reports
    /// a Lua-configured bind as `__lua` with a function number, so it cannot say
    /// which element a key runs.
    Shortcuts,
    /// The **Wallpaper** page: hyprpaper's own config, generated from
    /// `~/.config/hypr-osd/wallpaper.conf` (see [`crate::wallpaper`]). A change is
    /// written at once and applied through hyprpaper's IPC, so the picture changes
    /// on screen without a restart; the splash and `ipc` need the daemon restarted,
    /// which the page's own button does.
    Wallpaper,
    /// The **Login screen** page: SDDM's hypr-osd theme, written into
    /// `~/.config/hypr/sddm-theme/theme.conf` from
    /// `~/.config/hypr-osd/login.conf`, with the background rendered from the
    /// wallpaper (see [`crate::login`]). Installing the theme into
    /// `/usr/share/sddm/` needs root, so the page says so instead of trying.
    Login,
}

/// One element's (or one Hyprland area's) page in the window.
pub struct Section {
    /// The config file this page's keys live in: `<file>.conf` under
    /// `~/.config/hypr-osd/`. The elements have one each; every page of
    /// the Hyprland tab shares [`crate::desktop::FILE`], because the whole
    /// desktop is handed to the compositor in one document. On an
    /// [`Applies::Element`] page this is also the element's name - the binary
    /// the Restart chip runs.
    pub file: &'static str,
    pub title: &'static str,
    /// The Lucide drawing the sidebar shows beside the title.
    pub icon: &'static str,
    /// One line about what the page covers, at the top of it.
    pub blurb: &'static str,
    pub applies: Applies,
    pub settings: &'static [Setting],
}

/// Where a shortcut's keys are read from - which is also the two groups the
/// Shortcuts page draws.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShortcutScope {
    /// Bound in Hyprland's own config (`~/.config/hypr/osd.lua`), so the keys
    /// reach an element from anywhere, even with no card up. This is the half a
    /// test can hold against the template.
    Hyprland,
    /// Read by a card itself, while it is up and owns the keyboard. Hyprland never
    /// sees these keys - the card's own `on_keys` handler does.
    Card,
}

/// One keyboard shortcut this collection implements.
///
/// Not a [`Setting`]: nothing here is written to a file or edited. It is the
/// documentation of what the elements' keys are, kept in the same catalogue so a
/// page can be built from it and the template can be checked against it.
pub struct Shortcut {
    /// The keys, spelled the way Hyprland's config spells a bind
    /// (`SUPER + SHIFT + L`) - or, for a card's own keys, the way a person
    /// presses them (`Esc`, `1 - 9`).
    pub keys: &'static str,
    /// What pressing them does, in one line.
    pub action: &'static str,
    /// The sentence under it: which card shows what, or what the key needs to be
    /// worth pressing.
    pub help: &'static str,
    pub scope: ShortcutScope,
}

/// One tab of the window: a set of pages, and the label on its button.
pub struct Tab {
    /// What the `tab` verb calls it.
    pub id: &'static str,
    pub title: &'static str,
    /// One sentence for the button's tooltip.
    pub blurb: &'static str,
    pub sections: &'static [Section],
}

/// The window's tabs, in the order their buttons are drawn.
///
/// The first is this collection - one page per element, each with its own config
/// file. The second is the desktop the collection sits on: Hyprland's own
/// options, which belong together because they are not *about* any one element
/// and are applied to the compositor rather than to a program of ours. The third
/// is the desktop's three faces - the wallpaper, the lock screen and the login
/// screen - which are one tab because they are one picture seen three times. The
/// fourth is the machine under all of it: the energy policy that ties the
/// compositor's effects to the power profile (see [`crate::energy`]).
pub const TABS: &[Tab] = &[
    Tab {
        id: "osd",
        title: "Hypr OSD",
        blurb: "This collection: the bar, the popups and every card, one page each.",
        sections: OSD,
    },
    Tab {
        id: "hyprland",
        title: "Hyprland",
        blurb: "The desktop itself: Hyprland's own options, applied to the compositor \
                as you change them.",
        sections: DESKTOP,
    },
    Tab {
        id: "appearance",
        title: "Appearance",
        blurb: "The desktop's three faces - the wallpaper, the lock screen and the login \
                screen - and the picture they can share.",
        sections: APPEARANCE,
    },
    Tab {
        id: "system",
        title: "System",
        blurb: "The machine under the desktop: what its power profile is right now, and \
                the effects the collection drops while it is saving energy.",
        sections: SYSTEM,
    },
];

/// The **Appearance** tab: everything that is a picture rather than a program.
///
/// One page per face, in the order a session meets them - the login screen before
/// the wallpaper, the wallpaper behind the lock screen. All three can point at the
/// same image, and the Wallpaper page's own buttons are how that is done in one
/// click (see the settings element's `command`).
pub const APPEARANCE: &[Section] = &[
    Section {
        file: crate::wallpaper::FILE,
        title: "Wallpaper",
        icon: names::DISPLAY,
        blurb: "hyprpaper: the picture behind every window, how it fills a screen, and \
                the slideshow it can be.",
        applies: Applies::Wallpaper,
        settings: WALLPAPER,
    },
    Section {
        file: crate::lock::FILE,
        title: "Lock screen",
        icon: names::LOCK,
        blurb: "SUPER + L, and the session card's Lock row: the wallpaper under a sheet \
                of glass, the clock, the date and the field - every knob hyprlock has \
                for them.",
        applies: Applies::Lock,
        settings: LOCK,
    },
    Section {
        file: crate::login::FILE,
        title: "Login screen",
        icon: names::MONITOR,
        blurb: "SDDM, before Hyprland starts: the picture, the face and the palette the \
                greeter draws.",
        applies: Applies::Login,
        settings: LOGIN,
    },
];

/// The first tab: every element of the collection, in the order the sidebar shows
/// them - the bar first (it is the furniture), then the two popups that belong to
/// it, then the cards, then the two keyboard panels, then the settings window
/// itself.
pub const OSD: &[Section] = &[
    Section {
        file: "bar",
        title: "Bar",
        icon: names::GRID,
        blurb: "The furniture: workspaces, the window title, the media, the tray, the \
                pills and the clock, across the top of every screen.",
        applies: Applies::Element,
        settings: BAR,
    },
    Section {
        file: "island",
        title: "Island",
        icon: names::CLOCK,
        blurb: "The panel that unfolds from the bar's clock: what is playing, the \
                notifications, a calendar.",
        applies: Applies::Element,
        settings: ISLAND,
    },
    Section {
        file: "stats",
        title: "System popup",
        icon: names::CONTROLS,
        blurb: "The panel that unfolds from the bar's status pill: the machine's load, \
                the bluetooth radios, and the switches.",
        applies: Applies::Element,
        settings: STATS,
    },
    Section {
        file: "bluetooth",
        title: "Bluetooth manager",
        icon: names::BLUETOOTH,
        blurb: "The card the system popup's bluetooth tile opens: every paired and \
                nearby device, its battery and link, and the one action that fits it.",
        applies: Applies::Element,
        settings: BLUETOOTH,
    },
    Section {
        file: "volume",
        title: "Volume card",
        icon: names::VOLUME_HIGH,
        blurb: "The card the volume keys bring up. It owns the volume step, so this is \
                where one notch is measured.",
        applies: Applies::Element,
        settings: VOLUME,
    },
    Section {
        file: "media",
        title: "Media card",
        icon: names::MUSIC,
        blurb: "The card that appears when a new track starts. Watch-only: no key opens \
                it.",
        applies: Applies::Element,
        settings: MEDIA,
    },
    Section {
        file: "session",
        title: "Session card",
        icon: names::POWER,
        blurb: "Lock, suspend, log out, reboot and shut down, and the click-again rule \
                the destructive ones follow.",
        applies: Applies::Element,
        settings: SESSION,
    },
    Section {
        file: "switcher",
        title: "Window switcher",
        icon: names::LIST,
        blurb: "ALT + TAB: every window you have open, most recently used first.",
        applies: Applies::Element,
        settings: SWITCHER,
    },
    Section {
        file: "overview",
        title: "Workspace overview",
        icon: names::DISPLAY,
        blurb: "The whole desktop as a picture of itself, thumbnails and all.",
        applies: Applies::Element,
        settings: OVERVIEW,
    },
    Section {
        file: "launcher",
        title: "Launcher",
        icon: names::SEARCH,
        blurb: "The search card on SUPER + SPACE: installed applications and the files \
                in your own directories.",
        applies: Applies::Element,
        settings: LAUNCHER,
    },
    Section {
        file: "apps",
        title: "Applications panel",
        icon: names::GRID,
        blurb: "Every installed application, in a grid with drawers and a Pinned list, \
                opened by the bar's left-end button.",
        applies: Applies::Element,
        settings: APPS,
    },
    Section {
        file: "settings",
        title: "Settings",
        icon: names::SETTINGS,
        blurb: "This window. It reads its own config at start-up like every element, so \
                a change here takes a restart of the window to show.",
        applies: Applies::Element,
        settings: WINDOW,
    },
    Section {
        file: crate::touch::ELEMENT,
        title: "Touch",
        icon: names::MOUSE,
        blurb: "The touchscreen side: a finger tap opens the panels a hover opens, a \
                long press is the right click, a tablet gets an on-screen keyboard, and \
                the controls grow while the keyboard is folded away. Every element reads \
                this one file, so a change is a restart of the elements that are running.",
        applies: Applies::Touch,
        settings: TOUCH,
    },
    Section {
        file: "shortcuts",
        title: "Shortcuts",
        icon: names::KEYBOARD,
        blurb: "Every key this collection implements - the bound ones and the cards' \
                own.",
        applies: Applies::Shortcuts,
        // A list, not a form: the page's rows come from `SHORTCUTS`, and there is
        // no file of ours behind them.
        settings: &[],
    },
];

// --- the shortcuts page ----------------------------------------------------
//
// Every key the collection implements, in one table, for the Shortcuts page.
//
// The *bound* half is a mirror of `scripts/hyprland/osd.lua` - the template the
// installer writes to `~/.config/hypr/osd.lua`, which is where those keys really
// live. A test reads that file at compile time and requires the two to name each
// other's binds, so this table cannot quietly fall behind a key added there.
//
// The *card* half is not in that file at all: those keys are read by each
// element's own `on_keys` handler while its card is up, which is why Hyprland has
// nothing to say about them. They are listed here for the same reason as the rest
// - an overview of what the keyboard does on this desktop - and the element in
// question is named in the action, because half of these keys mean different
// things in different cards.
//
// `SUPER + SHIFT + M` (show the media card) and `SUPER + SHIFT + S` (this window)
// are deliberately absent: they are commented-out suggestions in the template,
// not binds, so they implement nothing today.

pub const SHORTCUTS: &[Shortcut] = &[
    Shortcut {
        keys: "SUPER + H",
        action: "Bar: put this window away",
        help: "Hyprland has no minimise, so the bar moves the focused window to \
               `special:minimized` and draws it as an icon in its tray. The window keeps \
               running.",
        scope: ShortcutScope::Hyprland,
    },
    Shortcut {
        keys: "SUPER + SHIFT + H",
        action: "Bar: bring the last window back",
        help: "The keyboard way to undo the row above. A window that was put away before \
               the bar was restarted comes back to the workspace you are on.",
        scope: ShortcutScope::Hyprland,
    },
    Shortcut {
        keys: "XF86AudioRaiseVolume",
        action: "Volume card: one step up",
        help: "The card owns the step: it runs wpctl, reads the sink back and shows what \
               it reports. It still works on the lock screen, where no card is drawn.",
        scope: ShortcutScope::Hyprland,
    },
    Shortcut {
        keys: "XF86AudioLowerVolume",
        action: "Volume card: one step down",
        help: "The same step, downwards - the size of a step is the Volume card page's \
               `step` key.",
        scope: ShortcutScope::Hyprland,
    },
    Shortcut {
        keys: "XF86AudioMute",
        action: "Volume card: mute or unmute",
        help: "Toggles the default sink and shows the card with the state it read back.",
        scope: ShortcutScope::Hyprland,
    },
    Shortcut {
        keys: "SUPER + SHIFT + L",
        action: "Session card: lock, suspend, log out, reboot, shut down",
        help: "The card's five rows are the only place these actions live. Pressing the \
               key again takes the card away; the destructive rows ask a second time.",
        scope: ShortcutScope::Hyprland,
    },
    Shortcut {
        keys: "XF86PowerOff",
        action: "Session card: the hardware power key",
        help: "Only reaches Hyprland when systemd-logind is told to ignore the key \
               (`HandlePowerKey=ignore`); until then logind acts on it itself. install.sh \
               reports the setting.",
        scope: ShortcutScope::Hyprland,
    },
    Shortcut {
        keys: "ALT + TAB",
        action: "Switcher: the next window, while ALT is held",
        help: "Hold ALT and tap TAB to walk the windows, most recently used first. \
               Letting go of ALT switches to the one that is highlighted.",
        scope: ShortcutScope::Hyprland,
    },
    Shortcut {
        keys: "ALT + SHIFT + TAB",
        action: "Switcher: the previous window",
        help: "The same walk in the other direction, for a hand that is already holding \
               ALT.",
        scope: ShortcutScope::Hyprland,
    },
    Shortcut {
        keys: "SUPER + SHIFT + TAB",
        action: "Overview: the whole desktop as a picture of itself",
        help: "Every workspace as a chip, every window as a tile. The same key takes the \
               card away again.",
        scope: ShortcutScope::Hyprland,
    },
    Shortcut {
        keys: "SUPER + SPACE",
        action: "Launcher: search applications and files",
        help: "Type to filter the installed applications and, a moment later, the files \
               in your own directories.",
        scope: ShortcutScope::Hyprland,
    },
    Shortcut {
        keys: "SUPER + L",
        action: "Lock the screen (hyprlock)",
        help: "`SUPER + SHIFT + L` is the session card, so bare `SUPER + L` gets the \
               lock. It refuses to start a second locker on an already locked screen.",
        scope: ShortcutScope::Hyprland,
    },
    Shortcut {
        keys: "TAB / SHIFT + TAB",
        action: "Switcher: walk the grid",
        help: "One tile at a time. The arrow keys do the same, and up/down move a whole \
               row.",
        scope: ShortcutScope::Card,
    },
    Shortcut {
        keys: "1 - 9",
        action: "Switcher: jump to that tile and switch to it",
        help: "The fastest way through a long list: the digit switches at once, without \
               releasing ALT.",
        scope: ShortcutScope::Card,
    },
    Shortcut {
        keys: "ENTER",
        action: "Switcher: switch now",
        help: "Commits the highlighted window without waiting for ALT to be released.",
        scope: ShortcutScope::Card,
    },
    Shortcut {
        keys: "ESC",
        action: "Switcher: walk away without switching",
        help: "Closes the card and leaves the focus where it was.",
        scope: ShortcutScope::Card,
    },
    Shortcut {
        keys: "ARROWS / H J K L",
        action: "Overview: walk the tiles",
        help: "Left/right and up/down move by tile; H/J/K/L are there because a \
               Hyprland user's hands already know them. TAB does the same as \
               left/right.",
        scope: ShortcutScope::Card,
    },
    Shortcut {
        keys: "1 - 9 / 0",
        action: "Overview: switch to that workspace",
        help: "0 is the tenth, as the example config numbers them. A workspace that does \
               not exist yet is created.",
        scope: ShortcutScope::Card,
    },
    Shortcut {
        keys: "ENTER",
        action: "Overview: go to the highlighted window",
        help: "Switches to its workspace and focuses it.",
        scope: ShortcutScope::Card,
    },
    Shortcut {
        keys: "ESC / Q",
        action: "Overview: leave",
        help: "Leaves the overview without going anywhere. `R` rebuilds the thumbnails \
               instead.",
        scope: ShortcutScope::Card,
    },
    Shortcut {
        keys: "UP / DOWN / PAGE UP / PAGE DOWN",
        action: "Launcher: walk the results",
        help: "Typing filters; these keys move the selection without leaving the field.",
        scope: ShortcutScope::Card,
    },
    Shortcut {
        keys: "ENTER",
        action: "Launcher: open the selected application or file",
        help: "Applications go through `gio launch`, so a `.desktop` entry's own Exec, \
               Terminal and TryExec are honoured.",
        scope: ShortcutScope::Card,
    },
    Shortcut {
        keys: "ESC",
        action: "Launcher: close the card",
        help: "The card also closes itself after `idle_close_ms` of silence, which is \
               what makes it safe across a screen lock.",
        scope: ShortcutScope::Card,
    },
    Shortcut {
        keys: "ARROWS / PAGE UP / PAGE DOWN / HOME / END",
        action: "Applications panel: walk the grid",
        help: "With Ctrl held these keys belong to the search field instead, so a line \
               can be edited without the grid moving.",
        scope: ShortcutScope::Card,
    },
    Shortcut {
        keys: "TAB / SHIFT + TAB",
        action: "Applications panel: the next or previous drawer",
        help: "Walks the sidebar - All, the categories, and Pinned once something is \
               pinned.",
        scope: ShortcutScope::Card,
    },
    Shortcut {
        keys: "ENTER",
        action: "Applications panel: open the selected application",
        help: "The same `gio launch` the launcher uses.",
        scope: ShortcutScope::Card,
    },
    Shortcut {
        keys: "ESC",
        action: "Applications panel: close it",
        help: "The bar's left-end button does the same, and so does \
               `idle_close_ms` of silence.",
        scope: ShortcutScope::Card,
    },
    Shortcut {
        keys: "CTRL + TAB / CTRL + SHIFT + TAB",
        action: "Settings: the next or previous tab",
        help: "Walks the window's four tabs without leaving the keyboard. Each tab \
               remembers the page it was last on.",
        scope: ShortcutScope::Card,
    },
    Shortcut {
        keys: "ESC",
        action: "Settings: close this window",
        help: "One of the two keys this window takes for itself (the other is \
               Ctrl+Tab) - everything else belongs to the control that has the focus.",
        scope: ShortcutScope::Card,
    },
];

// --- the Hyprland tab ------------------------------------------------------
//
// Hyprland's own options, one key per *option path*: `general:gaps_in` is the
// key, because that is exactly what `hyprctl getoption` takes and what the
// generated Lua turns into `general = { gaps_in = 5 }`. The defaults here are
// Hyprland's own, so an unset key means "whatever Hyprland does by default" -
// and only the keys that are set are ever written into the document, so this
// tab cannot quietly rewrite a machine's hand-written config.
//
// The paths are verified against a running compositor: one it does not know
// answers nothing, and a page's rows show what Hyprland currently has (see the
// element's `fields_of`), which is how a value set in the user's own
// `hyprland.lua` stays visible here instead of being invisibly overridden.

pub const DESKTOP: &[Section] = &[
    Section {
        file: crate::desktop::FILE,
        title: "Display",
        icon: names::DISPLAY,
        blurb: "Each connected output: its picture, where it sits, and how it is \
                scaled - and which one the cards appear on.",
        applies: Applies::Displays,
        settings: MONITOR_SETTINGS,
    },
    Section {
        file: crate::desktop::FILE,
        title: "General",
        icon: names::CONTROLS,
        blurb: "Gaps, borders, the window layout - how the windows are arranged on the \
                screen.",
        applies: Applies::Compositor,
        settings: GENERAL,
    },
    Section {
        file: crate::desktop::FILE,
        title: "Decoration",
        icon: names::BRIGHTNESS,
        blurb: "Corners, transparency, blur and shadows - what a window looks like.",
        applies: Applies::Compositor,
        settings: DECORATION,
    },
    Section {
        file: crate::desktop::FILE,
        title: "Input",
        icon: names::KEYBOARD,
        blurb: "The keyboard layout, the mouse, and the touchpad.",
        applies: Applies::Compositor,
        settings: INPUT,
    },
    Section {
        file: crate::desktop::FILE,
        title: "Animations",
        icon: names::SHUFFLE,
        blurb: "Whether windows animate, and which of the two dragging animations run.",
        applies: Applies::Compositor,
        settings: ANIMATION,
    },
    Section {
        file: crate::desktop::FILE,
        title: "Misc",
        icon: names::POWER_MODE,
        blurb: "The odds and ends: the splash, focus follow-ups, the cursor and the \
                middle-click paste.",
        applies: Applies::Compositor,
        settings: MISC,
    },
];

/// The fields of one monitor rule - the Display page, whose rows the window makes
/// once per connected output.
///
/// A key here is the *field's* name, not an option path: the window qualifies it
/// per output (`monitor:eDP-1:mode`) when it draws the page and writes it to
/// `desktop.conf`, and the generated document turns each fully-set output into one
/// `hl.monitor({...})` call (see [`crate::desktop`]). The defaults are Hyprland's
/// own, so an unset field means "leave that field alone" - except that a monitor
/// rule is all-or-nothing, so a field the file does not set is *filled* from what
/// the monitor is doing at the moment (see the element's `apply`).
pub const MONITOR_SETTINGS: &[Setting] = &[
    display(
        "mode",
        "Mode",
        "The resolution and refresh rate, out of the ones the monitor reports. \
         `preferred` asks for its own best mode.",
        "preferred",
        MonitorList::Mode,
    ),
    text(
        "position",
        "Position",
        "Where the output sits in the layout: `auto`, `auto-right`/`-left`/`-up`/`-down`, \
         or `XxY` (e.g. `1903x1440`).",
        "auto",
    ),
    display(
        "scale",
        "Scale",
        "How many physical pixels make one logical pixel - 2 doubles everything on a \
         HiDPI screen.",
        "auto",
        MonitorList::Scale,
    ),
    number(
        "transform",
        "Rotation",
        "Turn the output: 0 none, 1 90\u{b0}, 2 180\u{b0}, 3 270\u{b0}, 4-7 the same \
         flipped.",
        "0",
        0,
        7,
    ),
    display(
        "mirror",
        "Mirror",
        "Show another output's picture here as well, or `none`.",
        "none",
        MonitorList::Mirror,
    ),
    flag(
        "disabled",
        "Disable",
        "On, the output is switched off and its workspace moves to another screen.",
        false,
    ),
];

/// The key the Display page writes the collection's **primary display** under, in
/// `desktop.conf`.
///
/// It shares the file with the monitor rules and the option paths but is neither:
/// it is not an `hl.monitor` field and not a Hyprland option, and the compositor
/// never sees it. It is a setting *of this collection*, read back by the elements
/// themselves (see `crate::monitors::primary_connector`) so that the cards which
/// appear by themselves - the volume, media and session cards - always appear on
/// the screen the user chose instead of whichever one happened to be focused.
pub const PRIMARY_DISPLAY: &str = "primary";

/// The [`PRIMARY_DISPLAY`] value that means "no choice": keep following whatever
/// output is focused, which is what every card did before the setting existed.
///
/// It is a word rather than an empty value because the row is a dropdown, and an
/// empty value is how the window spells "the file does not set this key at all" -
/// a state a list control cannot pick back up from (see `view::choice_control`).
pub const PRIMARY_FOCUSED: &str = "focused";

/// The Display page's one row that is not about a single output: which output the
/// collection's notification cards appear on.
///
/// It is kept out of [`MONITOR_SETTINGS`] deliberately: that table is what the
/// window qualifies per output (`monitor:eDP-1:mode`) and what
/// [`crate::desktop::lua_document`] turns into `hl.monitor` calls, and a `primary`
/// field has no business in either. The row is drawn once, above the per-output
/// groups (see the settings element's `display_fields`).
pub const PRIMARY_DISPLAY_SETTING: Setting = display(
    PRIMARY_DISPLAY,
    "Primary display",
    "Where the volume, media and session cards appear. `Follow focus` keeps them \
     on whichever screen you are using; an output pins them there.",
    PRIMARY_FOCUSED,
    MonitorList::Primary,
);

// --- the bar ---------------------------------------------------------------

const BAR: &[Setting] = &[
    number("height", "Height", "The bar's own height, in pixels.", "40", 20, 120),
    number("margin_top", "Top margin", "How far below the screen's top edge it floats.", "8", 0, 80),
    number("margin_x", "Side margin", "The inset at the left and right ends.", "12", 0, 200),
    flag("exclusive", "Reserve its strip", "On, tiled windows are laid out below the bar; off, they slide under it.", true),
    text("output", "Output", "A connector name (eDP-1, DP-2, …) keeps the bar on that screen only. Empty means every monitor.", ""),
    // What is *on* the bar. Each one is a switch in the settings window, and
    // each one defaults to on: a part switched off is not built, not read and
    // not drawn (see `Shows` in `hypr-osd-bar`).
    flag("show_apps", "Applications button", "The grid button at the far left, which opens the applications panel.", true),
    flag("show_workspaces", "Workspaces", "The row of workspace pills.", true),
    flag("show_title", "Window title", "The focused window's title, beside the workspaces.", true),
    flag("show_clock", "Clock", "The clock at the bar's right end - and the handle that unfolds the island popup.", true),
    flag("show_media", "Media", "The player: cover, track and artist, in the middle of the bar.", true),
    flag("show_stats", "System info", "CPU, memory, temperature and the pending updates.", true),
    flag("show_status", "Status", "The link, the sound and the battery - and the handle that unfolds the system popup.", true),
    flag("show_tray", "Tray", "The application indicators. Off, the bar does not claim the tray's D-Bus names at all.", true),
    flag("show_minimised", "Put-away windows", "The windows you put away with SUPER + H, drawn in the tray's row.", true),
    flag("show_power", "Power button", "The session button, between the status pill and the clock.", true),
    number("workspaces", "Workspaces", "How many numbered workspaces the row keeps room for.", "5", 1, 30),
    number("title_width", "Title width", "How much room the focused window's title may take before it is ellipsised.", "320", 60, 2000),
    number("media_width", "Track width", "How much room the player's track may take before it is ellipsised.", "340", 60, 2000),
    text("clock_format", "Clock format", "The clock's top line, in strftime terms - `%H:%M` or `%a %H:%M`.", "%H:%M"),
    text("date_format", "Date format", "The small line under the time; empty gives a one-line clock.", "%a, %b %d"),
    text("battery", "Battery", "The battery to watch, by name. A machine with none gets no battery pill.", "BAT1"),
    text("adapter", "Adapter", "The mains supply that says whether the battery is charging.", "ADP1"),
    text("network_interface", "Wireless interface", "The interface the network pill reads, through `iw`.", "wlan0"),
    number("tray_icon_size", "Tray icon size", "How large the StatusNotifier indicators are drawn.", "18", 8, 48),
    text("terminal_command", "Terminal", "What the pills and popups open when they run something in a terminal.", "kitty"),
    text("updates_command", "Updates command", "What prints the pending-update list (`checkupdates`). Empty drops the count.", "checkupdates"),
    millis("tick_ms", "Heartbeat", "The clock's resolution, in milliseconds.", "1000", 100, 5000),
    millis("network_every_ms", "Network interval", "How often the network pill is re-read, in milliseconds.", "5000", 1000, 60000),
    millis("battery_every_ms", "Battery interval", "How often the battery is re-read, in milliseconds.", "30000", 1000, 300000),
    millis("stats_every_ms", "System interval", "How often the CPU/memory/temperature pill is re-read.", "1000", 500, 60000),
    millis("updates_every_ms", "Updates interval", "How often a stale update count is refreshed, in milliseconds.", "1800000", 60000, 3600000),
    millis("resync_every_ms", "Resync interval", "The safety net for a socket or a follower going quiet, in milliseconds.", "5000", 1000, 60000),
    text("volume_command", "Volume command", "The volume element the wheel asks, so one notch means one thing.", "hypr-osd-volume"),
    text("stats_command", "Popup command", "The element the status pill opens.", "hypr-osd-stats"),
    text("island_command", "Island command", "The element the clock's hover opens.", "hypr-osd-island"),
    text("apps_command", "Applications command", "The element the bar's left-end button opens.", "hypr-osd-apps"),
];

// --- touch (the machine's own page) -----------------------------------------

/// The defaults here are [`crate::touch`]'s, not an element's: `touch.conf` is
/// shared by every element and read at start-up (see `touch::settings`).
const TOUCH: &[Setting] = &[
    flag(
        "enabled",
        "Touch handling",
        "A finger tap opens the panels a hover opens (pinned, with a dimmed \
         tap-outside to close them), a long press is the right click, and the \
         controls grow on a tablet. Off, the touchscreen still clicks and drags \
         like a pointer.",
        true,
    ),
    text(
        "osk",
        "On-screen keyboard",
        "`auto` finds squeekboard, then wvkbd; `none` turns it off; anything else \
         is a command line (`wvkbd-mobintl -H 300`).",
        "auto",
    ),
    number(
        "osk_height",
        "Keyboard height",
        "How tall wvkbd is asked to be, in pixels. squeekboard sizes itself.",
        "260",
        80,
        600,
    ),
    number(
        "long_press_ms",
        "Long press",
        "How long a finger has to rest before it counts as a right click, in \
         milliseconds.",
        "500",
        200,
        2000,
    ),
];

// --- the island ------------------------------------------------------------

const ISLAND: &[Setting] = &[
    number(
        "bar_height",
        "Bar height",
        "The bar's height, so the panel can hang below it. Must match bar.conf.",
        "40",
        1,
        120,
    ),
    number(
        "bar_margin_top",
        "Bar top margin",
        "The bar's own top margin. Must match bar.conf.",
        "8",
        0,
        80,
    ),
    number(
        "hot_width",
        "Hover zone",
        "How wide a strip at the bar's right end counts as hovering the clock.",
        "120",
        40,
        1200,
    ),
    number(
        "gap",
        "Gap",
        "The distance between the bar's bottom edge and the panel's top.",
        "6",
        0,
        60,
    ),
    number(
        "left_width",
        "Left column",
        "The width of the column holding the player tile and the notifications.",
        "252",
        80,
        900,
    ),
    number(
        "right_width",
        "Right column",
        "The width of the calendar column.",
        "178",
        80,
        900,
    ),
    millis(
        "open_delay_ms",
        "Open delay",
        "How long the pointer rests on the clock before the panel opens.",
        "120",
        0,
        2000,
    ),
    millis(
        "close_delay_ms",
        "Close delay",
        "How long the pointer may be away before the panel closes.",
        "300",
        0,
        5000,
    ),
    millis(
        "poll_ms",
        "Pointer poll",
        "How often the pointer is sampled while the panel matters.",
        "50",
        10,
        1000,
    ),
    millis(
        "tick_ms",
        "Heartbeat",
        "The clock's resolution, in milliseconds.",
        "1000",
        100,
        5000,
    ),
    millis(
        "media_every_ms",
        "Media interval",
        "How often the player tile is re-read while the panel is up.",
        "700",
        100,
        10000,
    ),
    millis(
        "players_every_ms",
        "Players interval",
        "How often the list of players is re-read for the chips row.",
        "3000",
        500,
        60000,
    ),
    number(
        "notification_rows",
        "Notification rows",
        "How many notifications the tile lists, newest first (0 hides the list).",
        "3",
        0,
        5,
    ),
    text(
        "notifications_command",
        "Notifications command",
        "What the notification tile's buttons run (`swaync-client`).",
        "swaync-client",
    ),
];

// --- the system popup ------------------------------------------------------

const STATS: &[Setting] = &[
    number(
        "bar_height",
        "Bar height",
        "The bar's height, so the panel can hang below it. Must match bar.conf.",
        "40",
        1,
        120,
    ),
    number(
        "bar_margin_top",
        "Bar top margin",
        "The bar's own top margin. Must match bar.conf.",
        "8",
        0,
        80,
    ),
    number(
        "bar_margin_x",
        "Bar side margin",
        "The bar's own side margin, which the panel's right edge lines up with.",
        "12",
        0,
        200,
    ),
    number(
        "gap",
        "Gap",
        "The distance between the bar's bottom edge and the panel's top.",
        "6",
        0,
        60,
    ),
    number(
        "hot_width",
        "Fallback hover zone",
        "The strip at the bar's right end used when a hand-typed `open` sends no rectangle.",
        "240",
        40,
        1200,
    ),
    number(
        "left_width",
        "Left column",
        "The width of the load column: the gauges and the two radio tiles.",
        "260",
        80,
        900,
    ),
    number(
        "right_width",
        "Right column",
        "The width of the controls column and the updates tile.",
        "240",
        80,
        900,
    ),
    millis(
        "open_delay_ms",
        "Open delay",
        "How long the pointer rests on the pill before the panel opens.",
        "120",
        0,
        2000,
    ),
    millis(
        "close_delay_ms",
        "Close delay",
        "How long the pointer may be away before the panel closes.",
        "300",
        0,
        5000,
    ),
    millis(
        "poll_ms",
        "Pointer poll",
        "How often the pointer is sampled while the panel matters.",
        "50",
        10,
        1000,
    ),
    millis(
        "tick_ms",
        "Heartbeat",
        "The clock's resolution, in milliseconds.",
        "1000",
        100,
        5000,
    ),
    millis(
        "slow_every_ms",
        "Slow interval",
        "How often the readings that cost a command are re-read.",
        "5000",
        1000,
        60000,
    ),
    millis(
        "devices_every_ms",
        "Devices interval",
        "How often the bluetooth device list is re-read while the panel is open.",
        "10000",
        2000,
        120000,
    ),
    number(
        "max_devices",
        "Device rows",
        "How many device rows the list shows before it switches to a count.",
        "5",
        1,
        12,
    ),
    millis(
        "networks_every_ms",
        "Networks interval",
        "How often the networks in reach are re-read while the panel is open.",
        "15000",
        2000,
        120000,
    ),
    number(
        "max_networks",
        "Network rows",
        "How many network rows the wireless tile shows before it switches to a count.",
        "5",
        1,
        12,
    ),
    number(
        "scan_seconds",
        "Scan length",
        "How long the Scan button looks for devices, in seconds.",
        "12",
        2,
        60,
    ),
    text(
        "updates_command",
        "Updates command",
        "What prints the pending-update list. Empty turns the updates tile off.",
        "checkupdates",
    ),
    text(
        "network_interface",
        "Wireless interface",
        "The interface the network tile reports on and disconnects.",
        "wlan0",
    ),
    text(
        "terminal_command",
        "Terminal",
        "Where the buttons that open something put it (btop, nmtui, pacman).",
        "kitty",
    ),
    text(
        "settings_command",
        "Settings command",
        "What the header's gear runs - this settings window.",
        "hypr-osd-settings",
    ),
    text(
        "bluetooth_command",
        "Bluetooth command",
        "What the bluetooth tile's Manage chip runs - the manager card.",
        "hypr-osd-bluetooth",
    ),
];

// --- the bluetooth manager ------------------------------------------------

const BLUETOOTH: &[Setting] = &[
    number(
        "width",
        "Card width",
        "The card's width. It grows for nothing: every row is capped to fit it.",
        "640",
        420,
        1400,
    ),
    number(
        "list_height",
        "List height",
        "The height of the scrolling device list, below the header and filter.",
        "380",
        160,
        1200,
    ),
    number(
        "max_devices",
        "Device limit",
        "How many devices the card reads details for (one bluetoothctl info each).",
        "24",
        1,
        100,
    ),
    millis(
        "devices_every_ms",
        "Devices interval",
        "How often the controller and its devices are re-read while the card is open.",
        "5000",
        1000,
        120000,
    ),
    number(
        "scan_seconds",
        "Scan length",
        "How long the Scan button looks for devices, in seconds.",
        "12",
        2,
        60,
    ),
    millis(
        "idle_close_ms",
        "Idle close",
        "Close the card after this long with no key or pointer movement (0 disables it).",
        "120000",
        0,
        600000,
    ),
];

// --- the cards -------------------------------------------------------------

const VOLUME: &[Setting] = &[
    fraction(
        "step_percent",
        "Volume step",
        "What one press of a volume key moves the sink by, in percent.",
        "5.0",
        1.0,
        25.0,
    ),
    fraction(
        "max_percent",
        "Maximum",
        "The level the slider's top stands for. wpctl allows over-amplification.",
        "100.0",
        50.0,
        150.0,
    ),
    flag(
        "unmute_on_raise",
        "Unmute on raise",
        "Raising the volume of a muted sink also unmutes it.",
        true,
    ),
    millis(
        "duration_ms",
        "Stay up",
        "How long the card stays on screen, in milliseconds (0 keeps it while hovered).",
        "1400",
        0,
        30000,
    ),
    number(
        "width",
        "Width",
        "The card's width, in pixels.",
        "340",
        200,
        800,
    ),
    fraction(
        "bottom_margin",
        "Bottom margin",
        "How far up from the screen's bottom the card sits, as a share of its height.",
        "0.12",
        0.0,
        0.6,
    ),
];

const MEDIA: &[Setting] = &[
    millis(
        "duration_ms",
        "Stay up",
        "How long the card stays on screen, in milliseconds (0 keeps it while hovered).",
        "4000",
        0,
        30000,
    ),
    flag(
        "only_when_playing",
        "Only when playing",
        "On, a paused player does not bring the card up.",
        true,
    ),
    number(
        "width",
        "Width",
        "The card's width, in pixels.",
        "400",
        200,
        900,
    ),
    fraction(
        "bottom_margin",
        "Bottom margin",
        "How far up from the screen's bottom the card sits, as a share of its height.",
        "0.12",
        0.0,
        0.6,
    ),
];

const SESSION: &[Setting] = &[
    millis(
        "duration_ms",
        "Stay up",
        "How long the card stays on screen, in milliseconds (0 keeps it while hovered).",
        "8000",
        0,
        60000,
    ),
    number(
        "width",
        "Width",
        "The card's width in pixels; 0 lets the rows decide it.",
        "0",
        0,
        900,
    ),
    fraction(
        "bottom_margin",
        "Bottom margin",
        "How far up from the screen's bottom the card sits, as a share of its height.",
        "0.12",
        0.0,
        0.6,
    ),
];

// --- the wallpaper ---------------------------------------------------------
//
// hyprpaper's config is `~/.config/hypr/hyprpaper.conf`, generated from
// `~/.config/hypr-osd/wallpaper.conf` (see [`crate::wallpaper`]). Every option
// hyprpaper 0.8 has is a row: the three splash knobs, `ipc`, and the wallpaper
// block's `monitor`, `path`, `fit_mode`, `timeout`, `order` and `recursive` -
// the last three turn a directory into a slideshow.

pub const WALLPAPER: &[Setting] = &[
    path_folder(
        "path",
        "Picture",
        "The image hyprpaper draws, or a directory it walks as a slideshow (the folder \
         button picks a directory). Empty keeps whatever ~/.config/hypr/hyprpaper.conf \
         already names.",
        crate::wallpaper::DEFAULT_PATH,
    ),
    choice(
        "fit_mode",
        "Fit mode",
        "How the picture fills a screen: `cover` crops the overflow, `contain` letterboxes, \
         `tile` repeats it, `fill` stretches it.",
        crate::wallpaper::DEFAULT_FIT_MODE,
        crate::wallpaper::FIT_MODES,
    ),
    text(
        "monitor",
        "Monitor",
        "The output this block is for (e.g. `DP-1`); empty makes it the fallback for every \
         output.",
        crate::wallpaper::DEFAULT_MONITOR,
    ),
    number(
        "timeout",
        "Slide time",
        "Seconds between pictures when `path` is a directory.",
        crate::wallpaper::DEFAULT_TIMEOUT,
        1,
        86400,
    ),
    choice(
        "order",
        "Slide order",
        "Walk the directory in `random` order; `(none)` keeps the file order.",
        crate::wallpaper::DEFAULT_ORDER,
        &["random"],
    ),
    flag(
        "recursive",
        "Recursive",
        "Descend into subdirectories when `path` is a directory.",
        crate::wallpaper::DEFAULT_RECURSIVE,
    ),
    flag(
        "splash",
        "Splash",
        "Draw Hyprland's splash over a freshly loaded wallpaper. Read at start-up, so \
         restart hyprpaper after changing it.",
        crate::wallpaper::DEFAULT_SPLASH,
    ),
    number(
        "splash_offset",
        "Splash offset",
        "How far up the splash sits.",
        crate::wallpaper::DEFAULT_SPLASH_OFFSET,
        0,
        500,
    ),
    fraction(
        "splash_opacity",
        "Splash opacity",
        "How opaque the splash is.",
        crate::wallpaper::DEFAULT_SPLASH_OPACITY,
        0.0,
        1.0,
    ),
    flag(
        "ipc",
        "IPC",
        "Let `hyprctl hyprpaper` talk to the daemon - which is how a new picture is applied \
         without a restart. Read at start-up.",
        crate::wallpaper::DEFAULT_IPC,
    ),
];

// --- the lock screen -------------------------------------------------------
//
// hyprlock's config is `~/.config/hypr/hyprlock.conf`, generated from
// `~/.config/hypr-osd/lock.conf` and the theme's palette (see [`crate::lock`]):
// hyprlock cannot read CSS, so the colours are written out for it.
//
// Every option hyprlock 0.9 has for the widgets this screen uses is a row here -
// `general`, `auth`, `animations`, `background`, and the panel, clock, date, rule,
// lock mark, field and hint widgets. A change is live at the next lock; nothing
// has to be restarted.
//
// Public because [`crate::lock`] holds it against the file it generates (a test
// there): every row has to change that file.

pub const LOCK: &[Setting] = &[
    path(
        "wallpaper",
        "Wallpaper",
        "The picture behind the lock screen; empty follows the wallpaper hyprpaper draws, \
         so the desktop and the lock screen cannot disagree.",
        crate::lock::DEFAULT_WALLPAPER,
    ),
    text(
        "monitor",
        "Monitor",
        "The output the lock screen draws on; empty means every output.",
        crate::lock::DEFAULT_MONITOR,
    ),
    number(
        "blur_passes",
        "Blur passes",
        "How many times the blur runs; keep it at 1 or more, because 0 skips the rest of \
         the pipeline with it.",
        crate::lock::DEFAULT_BLUR_PASSES,
        1,
        10,
    ),
    number(
        "blur_size",
        "Blur size",
        "How far the picture is blurred: 1 is essentially untouched, 8-16 is frosted glass.",
        crate::lock::DEFAULT_BLUR_SIZE,
        0,
        32,
    ),
    fraction(
        "brightness",
        "Brightness",
        "Dim the picture, so a bright wallpaper does not glare at midnight.",
        crate::lock::DEFAULT_BRIGHTNESS,
        0.2,
        1.5,
    ),
    fraction(
        "contrast",
        "Contrast",
        "Lift or flatten the picture's contrast.",
        crate::lock::DEFAULT_CONTRAST,
        0.5,
        2.0,
    ),
    fraction(
        "vibrancy",
        "Vibrancy",
        "How much colour the picture keeps.",
        crate::lock::DEFAULT_VIBRANCY,
        0.0,
        1.0,
    ),
    fraction(
        "vibrancy_darkness",
        "Vibrancy darkness",
        "How dark the vibrancy pass is allowed to go; it is the pair to the row above.",
        crate::lock::DEFAULT_VIBRANCY_DARKNESS,
        0.0,
        1.0,
    ),
    fraction(
        "noise",
        "Grain",
        "A whisper of film grain over the picture.",
        crate::lock::DEFAULT_NOISE,
        0.0,
        0.05,
    ),
    number(
        "reload_time",
        "Reload time",
        "Seconds between re-reads of the picture; -1 reloads only on SIGUSR2.",
        crate::lock::DEFAULT_RELOAD_TIME,
        -1,
        86400,
    ),
    fraction(
        "crossfade_time",
        "Cross-fade",
        "Seconds to cross-fade between reloaded backgrounds; negative disables it.",
        crate::lock::DEFAULT_CROSSFADE_TIME,
        -1.0,
        10.0,
    ),
    flag(
        "hide_cursor",
        "Hide cursor",
        "No pointer on the lock screen - there is nothing on it to point at.",
        crate::lock::DEFAULT_HIDE_CURSOR,
    ),
    flag(
        "ignore_empty_input",
        "Ignore empty Enter",
        "An empty Enter is not a wrong password, so a stray one cannot spend a PAM \
         attempt.",
        crate::lock::DEFAULT_IGNORE_EMPTY_INPUT,
    ),
    flag(
        "immediate_render",
        "Draw immediately",
        "Draw the widgets before the picture is ready, so a lock looks deliberate a beat \
         earlier instead of showing the wallpaper appearing underneath it.",
        crate::lock::DEFAULT_IMMEDIATE_RENDER,
    ),
    flag(
        "text_trim",
        "Trim text",
        "Trim trailing whitespace from command output in labels.",
        crate::lock::DEFAULT_TEXT_TRIM,
    ),
    choice(
        "fractional_scaling",
        "Fractional scaling",
        "How the surface follows a fractional monitor scale: 0 off, 1 on, 2 auto.",
        crate::lock::DEFAULT_FRACTIONAL_SCALING,
        &["0", "1", "2"],
    ),
    choice(
        "screencopy_mode",
        "Screencopy",
        "How the background is captured: 0 GPU-accelerated, 1 CPU (slow).",
        crate::lock::DEFAULT_SCREENCOPY_MODE,
        &["0", "1"],
    ),
    millis(
        "fail_timeout",
        "Failure timeout",
        "How long a wrong password stays red, in milliseconds.",
        crate::lock::DEFAULT_FAIL_TIMEOUT,
        0,
        10000,
    ),
    flag(
        "animations",
        "Animations",
        "Fade the lock in and out, and animate the field's colours.",
        crate::lock::DEFAULT_ANIMATIONS,
    ),
    flag(
        "pam_enabled",
        "PAM",
        "Authenticate through PAM, which is how the password is checked.",
        crate::lock::DEFAULT_PAM_ENABLED,
    ),
    text(
        "pam_module",
        "PAM module",
        "The PAM service to ask (`/etc/pam.d/<name>`); `su` is the fallback when it is \
         missing.",
        crate::lock::DEFAULT_PAM_MODULE,
    ),
    flag(
        "fingerprint_enabled",
        "Fingerprint",
        "Authenticate in parallel through fprintd.",
        crate::lock::DEFAULT_FINGERPRINT_ENABLED,
    ),
    text(
        "fingerprint_ready_message",
        "Fingerprint ready",
        "What the field says while the reader waits for a finger.",
        crate::lock::DEFAULT_FINGERPRINT_READY_MESSAGE,
    ),
    text(
        "fingerprint_present_message",
        "Fingerprint scanning",
        "What the field says while a finger is being read.",
        crate::lock::DEFAULT_FINGERPRINT_PRESENT_MESSAGE,
    ),
    millis(
        "fingerprint_retry_delay",
        "Fingerprint retry",
        "How long to wait after an unrecognised finger before the next read, in \
         milliseconds.",
        crate::lock::DEFAULT_FINGERPRINT_RETRY_DELAY,
        0,
        5000,
    ),
    number(
        "panel_width",
        "Glass width",
        "How wide the sheet of dark glass behind the clock and the field is.",
        crate::lock::DEFAULT_PANEL_WIDTH,
        200,
        1600,
    ),
    number(
        "panel_height",
        "Glass height",
        "How tall that sheet is; the type inside is placed against its middle.",
        crate::lock::DEFAULT_PANEL_HEIGHT,
        200,
        1200,
    ),
    number(
        "panel_x",
        "Glass x",
        "Horizontal offset of the glass from the centre.",
        crate::lock::DEFAULT_PANEL_X,
        -4000,
        4000,
    ),
    number(
        "panel_y",
        "Glass y",
        "Vertical offset of the glass; **positive is up**, as everywhere on this screen.",
        crate::lock::DEFAULT_PANEL_Y,
        -4000,
        4000,
    ),
    number(
        "panel_rounding",
        "Glass rounding",
        "Corner radius of the glass.",
        crate::lock::DEFAULT_PANEL_ROUNDING,
        0,
        200,
    ),
    number(
        "panel_border",
        "Glass border",
        "Width of the glass's border, in pixels.",
        crate::lock::DEFAULT_PANEL_BORDER,
        0,
        50,
    ),
    fraction(
        "panel_opacity",
        "Glass opacity",
        "How opaque the glass is; it is the theme's `bg` at this opacity.",
        crate::lock::DEFAULT_PANEL_OPACITY,
        0.0,
        1.0,
    ),
    number(
        "panel_shadow_size",
        "Glass shadow",
        "How far the glass's shadow spreads.",
        crate::lock::DEFAULT_PANEL_SHADOW_SIZE,
        0,
        200,
    ),
    number(
        "panel_shadow_passes",
        "Shadow passes",
        "How many times the shadow is drawn; 0 disables it.",
        crate::lock::DEFAULT_PANEL_SHADOW_PASSES,
        0,
        10,
    ),
    fraction(
        "panel_shadow_boost",
        "Shadow boost",
        "How much the shadow's opacity is boosted.",
        crate::lock::DEFAULT_PANEL_SHADOW_BOOST,
        0.0,
        5.0,
    ),
    fraction(
        "panel_shadow_opacity",
        "Shadow opacity",
        "How dark the glass's shadow is.",
        crate::lock::DEFAULT_PANEL_SHADOW_OPACITY,
        0.0,
        1.0,
    ),
    font(
        "clock_font",
        "Font",
        "The face the clock, the date, the lock mark and the hint are drawn in.",
        crate::lock::DEFAULT_CLOCK_FONT,
    ),
    number(
        "clock_size",
        "Clock size",
        "The clock's font size.",
        crate::lock::DEFAULT_CLOCK_SIZE,
        8,
        400,
    ),
    number(
        "clock_x",
        "Clock x",
        "Horizontal offset of the clock, the rule and the lock mark.",
        crate::lock::DEFAULT_CLOCK_X,
        -4000,
        4000,
    ),
    number(
        "clock_y",
        "Clock y",
        "Vertical offset of the clock; positive is up.",
        crate::lock::DEFAULT_CLOCK_Y,
        -4000,
        4000,
    ),
    text(
        "clock_format",
        "Clock format",
        "The clock's `date` format. `%H:%M` gets the accent colon; anything else is handed \
         to `date` unchanged.",
        crate::lock::DEFAULT_CLOCK_FORMAT,
    ),
    flag(
        "clock_accent_colon",
        "Accent colon",
        "Colour the colon between hours and minutes with the theme's accent (only for \
         `%H:%M`).",
        crate::lock::DEFAULT_CLOCK_ACCENT_COLON,
    ),
    text(
        "date_format",
        "Date format",
        "The date line under the clock, in `date` terms.",
        crate::lock::DEFAULT_DATE_FORMAT,
    ),
    number(
        "date_size",
        "Date size",
        "The date's font size.",
        crate::lock::DEFAULT_DATE_SIZE,
        6,
        300,
    ),
    number(
        "date_x",
        "Date x",
        "Horizontal offset of the date.",
        crate::lock::DEFAULT_DATE_X,
        -4000,
        4000,
    ),
    number(
        "date_y",
        "Date y",
        "Vertical offset of the date; positive is up.",
        crate::lock::DEFAULT_DATE_Y,
        -4000,
        4000,
    ),
    flag(
        "date_uppercase",
        "Date in caps",
        "Set the date in capitals, the way the tuned screen did.",
        crate::lock::DEFAULT_DATE_UPPERCASE,
    ),
    flag(
        "show_rule",
        "Accent rule",
        "Draw the short accent line where the clock and the field meet.",
        crate::lock::DEFAULT_SHOW_RULE,
    ),
    number(
        "rule_width",
        "Rule width",
        "Width of the accent rule.",
        crate::lock::DEFAULT_RULE_WIDTH,
        0,
        2000,
    ),
    number(
        "rule_height",
        "Rule height",
        "Height of the accent rule.",
        crate::lock::DEFAULT_RULE_HEIGHT,
        0,
        50,
    ),
    number(
        "rule_y",
        "Rule y",
        "Vertical offset of the rule; positive is up.",
        crate::lock::DEFAULT_RULE_Y,
        -4000,
        4000,
    ),
    number(
        "rule_rounding",
        "Rule rounding",
        "Corner radius of the rule.",
        crate::lock::DEFAULT_RULE_ROUNDING,
        0,
        100,
    ),
    fraction(
        "rule_opacity",
        "Rule opacity",
        "How opaque the accent rule is.",
        crate::lock::DEFAULT_RULE_OPACITY,
        0.0,
        1.0,
    ),
    flag(
        "show_lock_icon",
        "Lock mark",
        "Draw the lock mark above the password field.",
        crate::lock::DEFAULT_SHOW_LOCK_ICON,
    ),
    text(
        "lock_icon",
        "Lock glyph",
        "The glyph drawn as the lock mark. hyprlock draws text, so this is a Nerd Font \
         character rather than a Lucide drawing.",
        crate::lock::DEFAULT_LOCK_ICON,
    ),
    number(
        "lock_size",
        "Lock size",
        "The lock mark's font size.",
        crate::lock::DEFAULT_LOCK_SIZE,
        6,
        300,
    ),
    number(
        "lock_y",
        "Lock y",
        "Vertical offset of the lock mark; positive is up.",
        crate::lock::DEFAULT_LOCK_Y,
        -4000,
        4000,
    ),
    font(
        "field_font",
        "Field font",
        "The face the password field is drawn in.",
        crate::lock::DEFAULT_FIELD_FONT,
    ),
    number(
        "field_width",
        "Field width",
        "Width of the password field.",
        crate::lock::DEFAULT_FIELD_WIDTH,
        80,
        2000,
    ),
    number(
        "field_height",
        "Field height",
        "Height of the password field.",
        crate::lock::DEFAULT_FIELD_HEIGHT,
        20,
        500,
    ),
    number(
        "field_x",
        "Field x",
        "Horizontal offset of the password field.",
        crate::lock::DEFAULT_FIELD_X,
        -4000,
        4000,
    ),
    number(
        "field_y",
        "Field y",
        "Vertical offset of the password field; positive is up.",
        crate::lock::DEFAULT_FIELD_Y,
        -4000,
        4000,
    ),
    number(
        "field_rounding",
        "Field rounding",
        "Corner radius of the password field.",
        crate::lock::DEFAULT_FIELD_ROUNDING,
        0,
        200,
    ),
    fraction(
        "field_tint",
        "Field tint",
        "How strong the accent tint inside the field is.",
        crate::lock::DEFAULT_FIELD_TINT,
        0.0,
        1.0,
    ),
    number(
        "field_outline",
        "Field outline",
        "Thickness of the field's outline; 0 tints the inside instead.",
        crate::lock::DEFAULT_FIELD_OUTLINE,
        0,
        50,
    ),
    fraction(
        "field_outline_accent",
        "Outline accent",
        "The first colour of the outline gradient: the accent at this opacity.",
        crate::lock::DEFAULT_FIELD_OUTLINE_ACCENT,
        0.0,
        1.0,
    ),
    fraction(
        "field_outline_accent2",
        "Outline accent2",
        "The second colour of the outline gradient: the accent2 at this opacity.",
        crate::lock::DEFAULT_FIELD_OUTLINE_ACCENT2,
        0.0,
        1.0,
    ),
    fraction(
        "dots_size",
        "Dot size",
        "Size of the password dots.",
        crate::lock::DEFAULT_DOTS_SIZE,
        0.001,
        0.8,
    ),
    fraction(
        "dots_spacing",
        "Dot spacing",
        "Space between the password dots.",
        crate::lock::DEFAULT_DOTS_SPACING,
        -1.0,
        1.0,
    ),
    flag(
        "dots_center",
        "Centre dots",
        "Centre the dots in the field instead of aligning them left.",
        crate::lock::DEFAULT_DOTS_CENTER,
    ),
    number(
        "dots_rounding",
        "Dot rounding",
        "Corner radius of a dot; -1 is fully round.",
        crate::lock::DEFAULT_DOTS_ROUNDING,
        -1,
        100,
    ),
    text(
        "dots_text_format",
        "Dot text",
        "A character to draw instead of a dot; empty keeps the rounded rectangles.",
        crate::lock::DEFAULT_DOTS_TEXT_FORMAT,
    ),
    text(
        "placeholder_text",
        "Placeholder",
        "What the empty field says. Pango markup is allowed.",
        crate::lock::DEFAULT_PLACEHOLDER_TEXT,
    ),
    text(
        "check_text",
        "Checking text",
        "What the field says while PAM is thinking. `$PAMPROMPT` and friends are \
         substituted.",
        crate::lock::DEFAULT_CHECK_TEXT,
    ),
    text(
        "fail_text",
        "Failure text",
        "What the field says after a wrong password. `$FAIL` and `$ATTEMPTS` are \
         substituted.",
        crate::lock::DEFAULT_FAIL_TEXT,
    ),
    flag(
        "fade_on_empty",
        "Fade when empty",
        "Hide the field while the buffer is empty, so an empty box does not say 'type \
         here'.",
        crate::lock::DEFAULT_FADE_ON_EMPTY,
    ),
    millis(
        "fade_timeout",
        "Fade delay",
        "How long the field waits after the buffer empties before fading, in milliseconds.",
        crate::lock::DEFAULT_FADE_TIMEOUT,
        0,
        10000,
    ),
    flag(
        "hide_input",
        "Hide input",
        "Draw a swaylock-style indicator instead of dots.",
        crate::lock::DEFAULT_HIDE_INPUT,
    ),
    flag(
        "swap_font_color",
        "Swap font colour",
        "Swap the font and inner colours when the field changes state.",
        crate::lock::DEFAULT_SWAP_FONT_COLOR,
    ),
    flag(
        "invert_numlock",
        "Invert numlock",
        "Change the colour when numlock is off rather than on.",
        crate::lock::DEFAULT_INVERT_NUMLOCK,
    ),
    text(
        "hint_text",
        "Hint",
        "The line under the field at rest. `$ATTEMPTS[…]` hides it once there is something \
         to report; empty removes the label.",
        crate::lock::DEFAULT_HINT_TEXT,
    ),
    number(
        "hint_size",
        "Hint size",
        "The hint's font size.",
        crate::lock::DEFAULT_HINT_SIZE,
        6,
        200,
    ),
    number(
        "hint_x",
        "Hint x",
        "Horizontal offset of the hint.",
        crate::lock::DEFAULT_HINT_X,
        -4000,
        4000,
    ),
    number(
        "hint_y",
        "Hint y",
        "Vertical offset of the hint; positive is up.",
        crate::lock::DEFAULT_HINT_Y,
        -4000,
        4000,
    ),
];

// --- the login screen ------------------------------------------------------
//
// SDDM's hypr-osd theme, written into `~/.config/hypr/sddm-theme/theme.conf` from
// `~/.config/hypr-osd/login.conf` (see [`crate::login`]). The palette is mirrored
// rather than read, because the greeter is Qt and cannot read CSS - the same
// reason the lock screen's colours are generated.
//
// The rendering keys (blur, dim, saturation, size) are not theme.conf keys: they
// are what `magick` does to the wallpaper to produce the background image the
// greeter draws.

pub const LOGIN: &[Setting] = &[
    path(
        "background",
        "Picture",
        "The picture behind the login; empty follows the wallpaper hyprpaper draws.",
        crate::login::DEFAULT_BACKGROUND,
    ),
    flag(
        "render",
        "Render background",
        "Blur and dim the picture into the theme's own image. Off hands the greeter the \
         file directly, which needs it to be readable by the `sddm` user.",
        crate::login::DEFAULT_RENDER,
    ),
    number(
        "blur",
        "Blur",
        "How far the rendered background is blurred (magick's `-blur 0xN`).",
        crate::login::DEFAULT_BLUR,
        0,
        64,
    ),
    number(
        "dim",
        "Dim",
        "How much the rendered background is dimmed, in percent (magick's `-modulate`).",
        crate::login::DEFAULT_DIM,
        0,
        200,
    ),
    number(
        "saturation",
        "Saturation",
        "Colour saturation of the rendered background, in percent.",
        crate::login::DEFAULT_SATURATION,
        0,
        300,
    ),
    number(
        "render_width",
        "Image width",
        "Width the background is rendered at.",
        crate::login::DEFAULT_RENDER_WIDTH,
        512,
        7680,
    ),
    number(
        "render_height",
        "Image height",
        "Height the background is rendered at.",
        crate::login::DEFAULT_RENDER_HEIGHT,
        512,
        4320,
    ),
    font(
        "font",
        "Font",
        "The face the greeter draws its text in. It draws no glyphs, so any monospace \
         works.",
        crate::login::DEFAULT_FONT,
    ),
    color(
        "bg",
        "Background",
        "The greeter's background colour, for the bars and the sheet behind the form.",
        crate::login::DEFAULT_BG,
    ),
    color(
        "fg",
        "Foreground",
        "The greeter's main text colour.",
        crate::login::DEFAULT_FG,
    ),
    color(
        "fg_dim",
        "Dim text",
        "Secondary text: the session name, the hints.",
        crate::login::DEFAULT_FG_DIM,
    ),
    color(
        "fg_faint",
        "Faint text",
        "The quietest text: separators and placeholders.",
        crate::login::DEFAULT_FG_FAINT,
    ),
    color(
        "accent",
        "Accent",
        "The accent: the focused field, the login button.",
        crate::login::DEFAULT_ACCENT,
    ),
    color(
        "on_accent",
        "On accent",
        "Text drawn on top of the accent.",
        crate::login::DEFAULT_ON_ACCENT,
    ),
    color(
        "warn",
        "Warning",
        "Caps-lock and other warnings.",
        crate::login::DEFAULT_WARN,
    ),
    color(
        "crit",
        "Critical",
        "A failed login.",
        crate::login::DEFAULT_CRIT,
    ),
    number(
        "cursor_size",
        "Cursor size",
        "SDDM's cursor size, written into its drop-in. Installing the theme needs root, \
         so this is carried by `install.sh`.",
        crate::login::DEFAULT_CURSOR_SIZE,
        16,
        96,
    ),
];

// --- the keyboard panels ---------------------------------------------------

const SWITCHER: &[Setting] = &[
    number(
        "columns",
        "Columns",
        "How many window tiles fit in one row.",
        "5",
        1,
        10,
    ),
    number(
        "tile_width",
        "Tile width",
        "How wide one tile is - and so how much of a title fits in it.",
        "132",
        48,
        400,
    ),
    number(
        "icon_size",
        "Icon size",
        "How large an application's icon is drawn in a tile.",
        "72",
        16,
        160,
    ),
    millis(
        "idle_commit_ms",
        "Idle commit",
        "Commit the selection after this long with no key, in milliseconds (0 disables it).",
        "5000",
        0,
        60000,
    ),
];

const OVERVIEW: &[Setting] = &[
    number(
        "tile_min_height",
        "Smallest tile",
        "The smallest height a window tile may be scaled to.",
        "84",
        48,
        600,
    ),
    number(
        "tile_max_height",
        "Largest tile",
        "The largest height a window tile may be scaled to.",
        "300",
        48,
        900,
    ),
    number(
        "gap",
        "Gap",
        "The space between tiles, and between rows of them.",
        "18",
        0,
        64,
    ),
    number(
        "icon_size",
        "Icon size",
        "The stand-in icon's size, where a tile has no picture.",
        "64",
        16,
        160,
    ),
    flag(
        "thumbnails",
        "Thumbnails",
        "Off, the card is a workspace switcher drawn with application icons.",
        true,
    ),
    millis(
        "idle_close_ms",
        "Idle close",
        "Close the card after this long with no key or pointer movement (0 disables it).",
        "60000",
        0,
        300000,
    ),
];

const LAUNCHER: &[Setting] = &[
    number("width", "Width", "The search card's width, in pixels.", "640", 360, 1400),
    number("max_results", "Result rows", "How many rows the list holds. The card reserves all of them, so it keeps its size while you type.", "7", 1, 20),
    number("icon_size", "Icon size", "How large an application's or a file's icon is drawn.", "22", 12, 48),
    flag("files", "Search files", "Whether your own directories are searched at all.", true),
    millis("debounce_ms", "Typing debounce", "How long a keystroke waits before a file walk is started.", "120", 0, 2000),
    millis("idle_close_ms", "Idle close", "Close the card after this long with no key or pointer movement (0 disables it).", "60000", 0, 300000),
];

const APPS: &[Setting] = &[
    number(
        "width",
        "Width",
        "The panel's width, in pixels. The tile pane gets what the sidebar leaves.",
        "760",
        420,
        2000,
    ),
    number(
        "sidebar_width",
        "Sidebar width",
        "The width of the drawer list down the side.",
        "168",
        96,
        400,
    ),
    number(
        "grid_height",
        "Grid height",
        "The height of the tile pane. A whole number of rows reads best (`4*96 + 3*6`).",
        "402",
        120,
        1200,
    ),
    number(
        "tile_width",
        "Tile width",
        "How wide one application tile is.",
        "100",
        64,
        300,
    ),
    number(
        "tile_height",
        "Tile height",
        "How tall one application tile is.",
        "96",
        56,
        300,
    ),
    number(
        "icon_size",
        "Icon size",
        "How large an application's icon is drawn in a tile.",
        "40",
        16,
        96,
    ),
    number(
        "tile_gap",
        "Tile gap",
        "The space between tiles.",
        "6",
        0,
        40,
    ),
    number(
        "gap",
        "Gap",
        "The space between the bar's bottom edge and the panel's top.",
        "6",
        0,
        60,
    ),
    number(
        "bar_height",
        "Bar height",
        "The bar's height, so the panel can hang below it. Must match bar.conf.",
        "40",
        1,
        120,
    ),
    number(
        "bar_margin_top",
        "Bar top margin",
        "The bar's own top margin. Must match bar.conf.",
        "8",
        0,
        80,
    ),
    number(
        "bar_margin_x",
        "Bar side margin",
        "The bar's own side margin, which the panel's left edge lines up with.",
        "12",
        0,
        200,
    ),
    number(
        "hot_width",
        "Fallback hover zone",
        "The strip at the bar's left end used when a hand-typed `open` sends no rectangle.",
        "60",
        20,
        600,
    ),
    millis(
        "open_delay_ms",
        "Open delay",
        "How long the pointer rests on the button before the panel opens.",
        "120",
        0,
        2000,
    ),
    millis(
        "close_delay_ms",
        "Close delay",
        "How long the pointer may be away before the panel closes.",
        "300",
        0,
        5000,
    ),
    millis(
        "poll_ms",
        "Pointer poll",
        "How often the pointer is sampled while the panel matters.",
        "50",
        10,
        1000,
    ),
    millis(
        "idle_close_ms",
        "Idle close",
        "Close the panel after this long with no key or pointer movement (0 disables it).",
        "60000",
        0,
        300000,
    ),
];

const WINDOW: &[Setting] = &[
    number(
        "width",
        "Width",
        "This card's width, in pixels.",
        "760",
        480,
        1600,
    ),
    number(
        "sidebar_width",
        "Sidebar width",
        "The width of the list of elements down the side.",
        "190",
        96,
        400,
    ),
    number(
        "pane_height",
        "Pane height",
        "The height of the settings pane. It scrolls; the card keeps this size.",
        "380",
        160,
        1200,
    ),
    millis(
        "idle_close_ms",
        "Idle close",
        "Close the window after this long with no key or pointer movement (0 disables it).",
        "120000",
        0,
        900000,
    ),
    text(
        "bin_dir",
        "Binaries",
        "Where the element binaries live, so the restart buttons can find them.",
        "~/.local/bin",
    ),
    text(
        "terminal_command",
        "Terminal",
        "What a button that needs a terminal opens - the Login screen page's Install runs \
         `sudo` there, so the password is typed where the installer's output can be read.",
        "kitty",
    ),
];

// --- the Hyprland options the Hyprland tab offers --------------------------
//
// One key per Hyprland option path, and the default is Hyprland's own, so a key
// nobody has set is not written into the generated document at all.

const GENERAL: &[Setting] = &[
    number("general:gaps_in", "Inner gaps", "The gap between windows. Hyprland keeps this one as four numbers, so its current value reads as `5 5 5 5`.", "5", 0, 100),
    number("general:gaps_out", "Outer gaps", "The gap between a window and the screen's edge.", "20", 0, 200),
    number("general:border_size", "Border size", "The border drawn around a window.", "1", 0, 20),
    text("general:layout", "Layout", "Which tiling layout places the windows: `dwindle` or `master`.", "dwindle"),
    flag("general:snap:enabled", "Snap windows", "Snap a window being moved to its neighbours' edges.", false),
    flag("general:resize_on_border", "Resize on border", "Dragging a window's border resizes it.", false),
    flag("general:no_focus_fallback", "No focus fallback", "On, the focus does not walk to another window when the one it was on closes.", false),
];

const DECORATION: &[Setting] = &[
    number(
        "decoration:rounding",
        "Corner rounding",
        "How round a window's corners are.",
        "0",
        0,
        40,
    ),
    fraction(
        "decoration:active_opacity",
        "Active opacity",
        "The focused window's opacity; 1.0 is opaque.",
        "1.0",
        0.1,
        1.0,
    ),
    fraction(
        "decoration:inactive_opacity",
        "Inactive opacity",
        "The opacity of every window that is not focused.",
        "1.0",
        0.1,
        1.0,
    ),
    flag(
        "decoration:blur:enabled",
        "Blur",
        "Blur what is behind a translucent window.",
        false,
    ),
    number(
        "decoration:blur:size",
        "Blur size",
        "The blur's radius.",
        "8",
        1,
        20,
    ),
    number(
        "decoration:blur:passes",
        "Blur passes",
        "How many passes of the blur run - more is smoother and dearer.",
        "1",
        1,
        10,
    ),
    fraction(
        "decoration:blur:vibrancy",
        "Vibrancy",
        "How much colour the blur keeps; 0.0 is grey.",
        "0.17",
        0.0,
        2.0,
    ),
    flag(
        "decoration:shadow:enabled",
        "Shadows",
        "Draw a shadow under a window.",
        false,
    ),
    number(
        "decoration:shadow:range",
        "Shadow range",
        "How far the shadow reaches.",
        "4",
        1,
        40,
    ),
    number(
        "decoration:shadow:render_power",
        "Shadow strength",
        "How dark the shadow is (1 to 4).",
        "3",
        1,
        4,
    ),
    flag(
        "decoration:dim_inactive",
        "Dim inactive",
        "Dim every window that is not focused.",
        false,
    ),
    fraction(
        "decoration:dim_strength",
        "Dim strength",
        "How much an unfocused window is dimmed.",
        "0.5",
        0.0,
        1.0,
    ),
];

const INPUT: &[Setting] = &[
    xkb(
        "input:kb_layout",
        "Keyboard layout",
        "The layout the keyboard types (`de`, `ch`, `us`, …) - the list is this \
         machine's own, so a name that is not in it cannot be picked. For several \
         layouts, write them comma-separated in desktop.conf.",
        "us",
        Keymap::Layout,
    ),
    xkb(
        "input:kb_variant",
        "Layout variant",
        "A variant *of the layout above* (`ch` makes a German layout Swiss German): \
         the list is that layout's own variants.",
        "",
        Keymap::Variant,
    ),
    xkb(
        "input:kb_model",
        "Keyboard model",
        "The XKB model of the keyboard (`pc105`, `apple`, …); empty detects it.",
        "",
        Keymap::Model,
    ),
    xkb(
        "input:kb_options",
        "Layout options",
        "XKB options (`grp:alt_shift_toggle` switches layouts with both Alts). For \
         several, write them comma-separated in desktop.conf.",
        "",
        Keymap::Option,
    ),
    number("input:follow_mouse", "Follow mouse", "Where the focus goes when the pointer moves: 0 never, 1 always, 2 on click, 3 click and raise.", "1", 0, 3),
    fraction("input:sensitivity", "Pointer sensitivity", "How fast the pointer moves (-1.0 to 1.0; 0 uses the device's own).", "0.0", -1.0, 1.0),
    choice(
        "input:accel_profile",
        "Acceleration profile",
        "How the pointer's acceleration curve is drawn; empty leaves it to the device.",
        "",
        &["adaptive", "flat"],
    ),
    flag("input:force_no_accel", "No acceleration", "Ignore the pointer's acceleration entirely.", false),
    flag("input:numlock_by_default", "Numlock on", "Turn numlock on at start-up.", false),
    flag("input:touchpad:natural_scroll", "Natural scrolling", "Scroll the way a touch screen does.", false),
    flag("input:touchpad:disable_while_typing", "Disable while typing", "Ignore the touchpad for a moment after a key press.", true),
    fraction("input:touchpad:scroll_factor", "Touchpad scroll speed", "A multiplier on the touchpad's scroll distance.", "1.0", 0.1, 5.0),
    flag("input:touchpad:clickfinger_behavior", "Clickfinger", "Two-finger click is the right button, three the middle one.", false),
];

const ANIMATION: &[Setting] = &[
    flag(
        "animations:enabled",
        "Animations",
        "Off, windows appear and move instantly.",
        true,
    ),
    flag(
        "misc:animate_manual_resizes",
        "Animate resizes",
        "Animate a resize you drag by hand - Hyprland leaves this off, because it is busy.",
        false,
    ),
    flag(
        "misc:animate_mouse_windowdragging",
        "Animate dragging",
        "Animate a window dragged with the mouse.",
        false,
    ),
];

const MISC: &[Setting] = &[
    flag(
        "misc:disable_hyprland_logo",
        "No logo",
        "Do not draw Hyprland's logo over an empty workspace.",
        false,
    ),
    flag(
        "misc:disable_splash_rendering",
        "No splash",
        "Skip the splash screen at start-up.",
        false,
    ),
    flag(
        "misc:focus_on_activate",
        "Focus on activate",
        "Let an application take the focus when it asks for it.",
        false,
    ),
    flag(
        "misc:key_press_enables_dpms",
        "Key press wakes",
        "A key press wakes the screens, not only mouse movement.",
        false,
    ),
    flag(
        "misc:middle_click_paste",
        "Middle-click paste",
        "Paste the primary selection on the middle mouse button.",
        true,
    ),
    flag(
        "misc:disable_autoreload",
        "No autoreload",
        "Ignore changes to the config file; a reload has to be asked for.",
        false,
    ),
    fraction(
        "cursor:inactive_timeout",
        "Cursor hide delay",
        "Hide the cursor after this many seconds of stillness (0 keeps it on screen).",
        "0.0",
        0.0,
        60.0,
    ),
    number(
        "cursor:no_hardware_cursors",
        "Hardware cursors",
        "Draw the cursor in the hardware plane: 0 on, 1 off, 2 automatic.",
        "2",
        0,
        2,
    ),
];

// --- the System tab --------------------------------------------------------
//
// One page, and it is not about Hyprland's options or about one element's own
// keys: it is the *policy* that ties the two together - which power profile means
// "save energy", which of the compositor's expensive effects are worth dropping
// while it is set, and the live readings that say whether it is working. The keys
// live in `~/.config/hypr-osd/energy.conf`, and the `hypr-osd-energy` daemon is
// what reads them and moves the compositor's options (see `crate::energy`).

pub const SYSTEM: &[Section] = &[Section {
    file: "energy",
    title: "Energy Saving",
    icon: names::POWER_MODE,
    blurb: "Drop the desktop's expensive effects while the machine saves energy.",
    applies: Applies::Energy,
    settings: ENERGY,
}];

/// The two buttons the Energy page carries. They are not settings: one asks the
/// daemon to save now, the other to put the remembered values back, and neither
/// writes a key - so they are [`Kind::Actions`] rather than a value the file
/// could hold.
pub const ENERGY_ACTIONS: &[Command] = &[
    Command {
        id: "apply",
        label: "Save now",
        icon: names::POWER_MODE,
    },
    Command {
        id: "restore",
        label: "Restore",
        icon: names::ROTATE_CW,
    },
];

/// The energy policy, one row per key of `energy.conf`.
///
/// The eight `disable_*`/`flatten_*`/`square_*` switches are the
/// [`crate::energy::switches`] - every one of them must be here, and a test holds
/// the two tables together so an effect cannot arrive without its switch (or with
/// one the window does not draw).
const ENERGY: &[Setting] = &[
    flag(
        "enabled",
        "Energy saving",
        "On, the collection follows the power profile: the effects below are dropped \
         while it is in the profile named next, and put back when it is not.",
        crate::energy::DEFAULT_ENABLED,
    ),
    choice(
        "profile",
        "Trigger profile",
        "The profile that counts as saving energy. Which profile the machine is in is \
         set from the system popup's POWER switch.",
        crate::energy::DEFAULT_PROFILE,
        &crate::energy::PROFILES,
    ),
    flag(
        "disable_animations",
        "Animations",
        "Windows appear and move instantly instead of easing into place.",
        true,
    ),
    flag(
        "disable_resize_animation",
        "Resize animation",
        "Dragging a window's edge resizes it without animating.",
        true,
    ),
    flag(
        "disable_drag_animation",
        "Drag animation",
        "A window dragged with the mouse does not animate.",
        true,
    ),
    flag(
        "disable_blur",
        "Blur",
        "Stop blurring what is behind a translucent window - the heaviest effect of \
         the lot on an integrated chip.",
        true,
    ),
    flag(
        "disable_shadows",
        "Shadows",
        "Draw no drop shadow under a window.",
        true,
    ),
    flag(
        "disable_dim_inactive",
        "Dim inactive",
        "Do not dim the windows that are not focused.",
        true,
    ),
    flag(
        "flatten_opacity",
        "Transparency",
        "Make every window fully opaque, focused or not - one switch for both \
         opacities.",
        true,
    ),
    flag(
        "square_corners",
        "Rounded corners",
        "Square the window corners off.",
        true,
    ),
    millis(
        "poll_ms",
        "Check interval",
        "How often the power profile is re-read, in milliseconds.",
        "4000",
        500,
        60000,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn sections() -> impl Iterator<Item = &'static Section> {
        TABS.iter().flat_map(|tab| tab.sections.iter())
    }

    /// Every tab names itself, has pages to show, and says what it covers.
    #[test]
    fn every_tab_is_usable() {
        for tab in TABS {
            assert!(!tab.id.is_empty(), "a tab has no id");
            assert!(!tab.title.is_empty(), "the `{}` tab has no title", tab.id);
            assert!(!tab.blurb.is_empty(), "the `{}` tab has no blurb", tab.id);
            assert!(
                !tab.sections.is_empty(),
                "the `{}` tab has no pages",
                tab.id
            );
        }
        // The first tab is this collection, the second the desktop it sits on, the
        // third the desktop's three faces, and the fourth the machine under both;
        // the window's buttons are drawn in this order.
        let ids: Vec<&str> = TABS.iter().map(|tab| tab.id).collect();
        assert_eq!(ids, vec!["osd", "hyprland", "appearance", "system"]);
    }

    /// Every page names a config file, has something to show, and wears an icon
    /// the element vocabulary actually carries.
    ///
    /// "Something" is its settings - except on the Shortcuts page, which is a
    /// list rather than a form: its rows come from [`SHORTCUTS`], so there is no
    /// key for it to name and no file for it to write.
    #[test]
    fn every_section_is_usable() {
        for section in sections() {
            assert!(!section.file.is_empty(), "{} has no file", section.title);
            assert!(
                !section.settings.is_empty() || section.applies == Applies::Shortcuts,
                "{} has nothing to show",
                section.title
            );
            assert!(!section.blurb.is_empty(), "{} has no blurb", section.title);
            assert!(
                names::ALL.iter().any(|(_, icon)| *icon == section.icon),
                "{}'s icon `{}` is not in the vocabulary",
                section.title,
                section.icon
            );
        }
    }

    /// A key appears once per page: two rows for one key would write the same
    /// line twice, and the second write would win invisibly.
    #[test]
    fn keys_are_unique_within_a_section() {
        for section in sections() {
            let mut keys: Vec<&str> = section.settings.iter().map(|setting| setting.key).collect();
            let count = keys.len();
            keys.sort_unstable();
            keys.dedup();
            assert_eq!(keys.len(), count, "{} names a key twice", section.title);
        }
    }

    /// The default has to be *valid for its own kind*: the control is built
    /// around it, so a default the spinner cannot hold would put the window in a
    /// state it cannot draw.
    #[test]
    fn every_default_fits_its_kind() {
        for section in sections() {
            for setting in section.settings {
                let where_ = format!("{}.{}", section.file, setting.key);
                match setting.kind {
                    Kind::Bool => assert!(
                        matches!(setting.default, "true" | "false"),
                        "{where_}: a switch default has to be true or false"
                    ),
                    Kind::Int { min, max, .. } => {
                        let value: i32 = setting.default.parse().unwrap_or_else(|_| {
                            panic!("{where_}: `{}` is not a number", setting.default)
                        });
                        assert!(
                            (min..=max).contains(&value),
                            "{where_}: default {value} is outside {min}..={max}"
                        );
                    }
                    Kind::Fraction { min, max } => {
                        let value: f64 = setting.default.parse().unwrap_or_else(|_| {
                            panic!("{where_}: `{}` is not a fraction", setting.default)
                        });
                        assert!(
                            (min..=max).contains(&value),
                            "{where_}: default {value} is outside {min}..={max}"
                        );
                    }
                    Kind::Text => {}
                    // A path is a line of text with a file chooser beside it; any
                    // string is a value it can hold, and a folder path is just a
                    // path that names a directory.
                    Kind::Path | Kind::PathFolder => {}
                    // A colour is spelled `#rrggbb` (or with an alpha byte) and a
                    // font is a family name - both are text, but one of them has
                    // a shape the chooser depends on.
                    Kind::Color => assert!(
                        setting.default.starts_with('#')
                            && matches!(setting.default.len(), 4 | 7 | 9),
                        "{where_}: `{}` is not a #rrggbb colour",
                        setting.default
                    ),
                    Kind::Font => assert!(
                        !setting.default.is_empty(),
                        "{where_}: a font needs a family name"
                    ),
                    Kind::Choice(options) => assert!(
                        setting.default.is_empty() || options.contains(&setting.default),
                        "{where_}: `{}` is not one of {options:?}",
                        setting.default
                    ),
                    // The list belongs to the machine, so its contents cannot be
                    // checked here - but a *default* is still one name this table
                    // chose, and a comma or a space in it would be a list or a
                    // sentence rather than a name.
                    Kind::Keymap(_) => assert!(
                        !setting.default.contains([',', ' ']),
                        "{where_}: `{}` does not look like one XKB name",
                        setting.default
                    ),
                    // A display list is the monitor's own, and its default is a
                    // word this table picked (`preferred`, `auto`, `none`).
                    Kind::Monitor(_) => assert!(
                        !setting.default.is_empty() && !setting.default.contains([' ', ',']),
                        "{where_}: `{}` does not look like one display value",
                        setting.default
                    ),
                    // A reading and a row of buttons are made up while a page is
                    // drawn; neither is ever a key in a table here.
                    Kind::Info | Kind::Actions(_) => {
                        panic!("{where_}: a reading or an action is not a catalogue setting")
                    }
                }
            }
        }
    }

    /// The keyboard rows are *picked*, never typed: a layout name that is not in
    /// the machine's list is not a value Hyprland rejects at the door - the
    /// option is accepted and the keyboard is the thing that breaks.
    #[test]
    fn the_keyboard_rows_are_dropdowns() {
        let kind = |key: &str| {
            INPUT
                .iter()
                .find(|setting| setting.key == key)
                .unwrap_or_else(|| panic!("no `{key}` in the Input page"))
                .kind
        };
        assert_eq!(kind("input:kb_layout"), Kind::Keymap(Keymap::Layout));
        assert_eq!(kind("input:kb_variant"), Kind::Keymap(Keymap::Variant));
        assert_eq!(kind("input:kb_model"), Kind::Keymap(Keymap::Model));
        assert_eq!(kind("input:kb_options"), Kind::Keymap(Keymap::Option));
        assert_eq!(
            kind("input:accel_profile"),
            Kind::Choice(&["adaptive", "flat"])
        );
    }

    /// The file names are what the window uses for both the config file and the
    /// binary, so a typo here would write to a file nobody reads.
    #[test]
    fn the_osd_tab_covers_the_whole_collection() {
        let files: Vec<&str> = OSD.iter().map(|section| section.file).collect();
        for expected in [
            "bar",
            "island",
            "stats",
            "bluetooth",
            "volume",
            "media",
            "session",
            "switcher",
            "overview",
            "launcher",
            "apps",
            "settings",
        ] {
            assert!(
                files.contains(&expected),
                "the catalogue has no page for `{expected}`"
            );
        }
        // Every page of the first tab is an element's, so it can be restarted; the
        // last one is the list of keys, which belongs to no element of its own -
        // and the Touch page is the machine's, not an element's (it has a file,
        // but every element reads that one).
        for section in OSD {
            assert!(
                matches!(
                    section.applies,
                    Applies::Element | Applies::Shortcuts | Applies::Touch
                ),
                "{} is not an element's page",
                section.title
            );
        }
        assert_eq!(
            OSD.last().map(|section| section.applies),
            Some(Applies::Shortcuts),
            "the Hypr OSD tab has no Shortcuts page"
        );
    }

    /// The Shortcuts page is only as good as its table: every row has to say what
    /// the keys do, and every *bound* row has to be a bind in the file the
    /// installer writes.
    ///
    /// `include_str!` puts the template in the test binary, so this is a real
    /// comparison rather than a promise: a key added to `osd.lua` and forgotten
    /// here fails the build's tests, and so does a row naming a key nobody bound.
    /// The template's commented-out suggestions (`SUPER + SHIFT + M`, `SUPER +
    /// SHIFT + S`, `SUPER + A`) are not binds and are deliberately not listed.
    ///
    /// The one kind of bind the page does *not* list is a **switch** bind
    /// (`switch:on:<name>`): it is not a shortcut anybody presses - it fires when
    /// a device changes state, and the Type Cover's tablet-mode switch is what
    /// the collection binds to grow the bar for a finger. It is documented in the
    /// README's Touch section, where the tablet is, not among the keys.
    #[test]
    fn the_shortcut_page_matches_the_hyprland_template() {
        const TEMPLATE: &str = include_str!("../../../scripts/hyprland/osd.lua");
        // `hl.bind("KEY", …)` at the start of a line: the commented suggestions
        // begin with `--` (or with whitespace and `--`), so they do not match.
        let bound: Vec<&str> = TEMPLATE
            .lines()
            .filter_map(|line| line.trim().strip_prefix("hl.bind(\""))
            .filter_map(|rest| rest.split('"').next())
            .filter(|keys| !keys.starts_with("switch:"))
            .collect();
        assert!(
            !bound.is_empty(),
            "the template binds nothing - has the installer changed shape?"
        );
        let listed: Vec<&str> = SHORTCUTS
            .iter()
            .filter(|shortcut| shortcut.scope == ShortcutScope::Hyprland)
            .map(|shortcut| shortcut.keys)
            .collect();
        for keys in &bound {
            assert!(
                listed.contains(keys),
                "`{keys}` is bound in osd.lua but missing from the Shortcuts page"
            );
        }
        for keys in &listed {
            assert!(
                bound.contains(keys),
                "`{keys}` is on the Shortcuts page but nothing binds it"
            );
        }
        // The prose is what the page is *for*, so an empty row is a bug.
        for shortcut in SHORTCUTS {
            assert!(!shortcut.keys.is_empty(), "a shortcut has no keys");
            assert!(
                !shortcut.action.is_empty() && !shortcut.help.is_empty(),
                "`{}` does not say what it does",
                shortcut.keys
            );
        }
        // Both groups are drawn, so both have to have something in them.
        for scope in [ShortcutScope::Hyprland, ShortcutScope::Card] {
            assert!(
                SHORTCUTS.iter().any(|shortcut| shortcut.scope == scope),
                "the {scope:?} group is empty"
            );
        }
    }

    /// The Hyprland tab's keys are *option paths*: at least two `:`-separated
    /// parts, because `hl.config({ general = { gaps_in = 5 } })` has to walk into
    /// a table with them (see [`crate::desktop`]). A one-part key would write the
    /// option at the top level, where Hyprland has no options at all.
    ///
    /// The Display page is the one exception, and deliberately so: its keys are
    /// the *fields* of a monitor rule, which the window qualifies per output, so
    /// they are plain names. What it has in common with the rest of the tab is
    /// that it too is applied to the compositor at once.
    #[test]
    fn a_desktop_key_is_an_option_path() {
        for section in DESKTOP {
            assert_eq!(section.file, crate::desktop::FILE);
            match section.applies {
                Applies::Compositor => {
                    for setting in section.settings {
                        let parts: Vec<&str> = setting.key.split(':').collect();
                        assert!(
                            parts.len() >= 2,
                            "{}: `{}` is not an option path",
                            section.title,
                            setting.key
                        );
                        for part in parts {
                            assert!(
                                !part.is_empty()
                                    && part
                                        .chars()
                                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
                                "{}: `{}` has a part Hyprland does not use in an option name",
                                section.title,
                                setting.key
                            );
                        }
                    }
                }
                // A display field is a name, not a path: the output is what makes
                // it an address, and the window supplies that.
                Applies::Displays => {
                    for setting in section.settings {
                        assert!(
                            !setting.key.is_empty()
                                && setting
                                    .key
                                    .chars()
                                    .all(|c| c.is_ascii_alphanumeric() || c == '_'),
                            "{}: `{}` is not a monitor field name",
                            section.title,
                            setting.key
                        );
                    }
                }
                Applies::Element => panic!("{} is an element's page", section.title),
                Applies::Energy => panic!("{} is an energy page", section.title),
                Applies::Lock => panic!("{} is the lock screen's page", section.title),
                Applies::Wallpaper => panic!("{} is the wallpaper's page", section.title),
                Applies::Login => panic!("{} is the login screen's page", section.title),
                Applies::Touch => panic!("{} is the touch page", section.title),
                Applies::Shortcuts => panic!("{} is a list of keys", section.title),
            }
        }
        // The Display page is where the output list is drawn, so it has to be in
        // the Hyprland tab and it has to be offered.
        let display = DESKTOP
            .iter()
            .find(|section| section.applies == Applies::Displays)
            .expect("the Hyprland tab has no Display page");
        assert_eq!(display.title, "Display");
        assert!(std::ptr::eq(display.settings, MONITOR_SETTINGS));
    }

    /// The System tab is the machine's own, and its one page is the energy
    /// policy on the daemon's file.
    #[test]
    fn the_system_tab_holds_the_energy_page() {
        let system = TABS
            .iter()
            .find(|tab| tab.id == "system")
            .expect("there is no System tab");
        assert_eq!(SYSTEM.len(), 1);
        assert_eq!(SYSTEM[0].applies, Applies::Energy);
        assert_eq!(SYSTEM[0].file, "energy");
        assert!(std::ptr::eq(SYSTEM[0].settings, ENERGY));
        assert!(std::ptr::eq(system.sections, SYSTEM));
    }

    /// The Appearance tab is the desktop's three faces - the wallpaper, the lock
    /// screen and the login screen - and each is generated from its own file
    /// rather than read by an element at start-up.
    #[test]
    fn the_appearance_tab_holds_the_three_backgrounds() {
        let appearance = TABS
            .iter()
            .find(|tab| tab.id == "appearance")
            .expect("there is no Appearance tab");
        assert!(std::ptr::eq(appearance.sections, APPEARANCE));
        let applies: Vec<Applies> = APPEARANCE.iter().map(|section| section.applies).collect();
        assert_eq!(
            applies,
            vec![Applies::Wallpaper, Applies::Lock, Applies::Login]
        );
        let files: Vec<&str> = APPEARANCE.iter().map(|section| section.file).collect();
        assert_eq!(files, vec!["wallpaper", "lock", "login"]);
        assert!(std::ptr::eq(APPEARANCE[0].settings, WALLPAPER));
        assert!(std::ptr::eq(APPEARANCE[1].settings, LOCK));
        assert!(std::ptr::eq(APPEARANCE[2].settings, LOGIN));
    }

    /// The effect table and the page's switches name each other: every effect's
    /// setting is a key the page draws, and every switch-shaped key the page
    /// draws is one of the effects'. Without that, the daemon would watch a
    /// switch nobody can flip - or ignore one the user did flip.
    #[test]
    fn the_energy_page_carries_every_switch() {
        let switches = crate::energy::switches();
        let keys: Vec<&str> = ENERGY.iter().map(|setting| setting.key).collect();
        for name in &switches {
            assert!(
                keys.contains(name),
                "the Energy page has no row for `{name}`"
            );
        }
        for key in &keys {
            let policy = matches!(*key, "enabled" | "profile" | "poll_ms");
            assert!(
                policy || switches.contains(key),
                "`{key}` is neither a policy key nor an effect's switch"
            );
        }
        // A switch row is a switch, and the trigger is one of the profiles.
        for setting in ENERGY {
            if switches.contains(&setting.key) {
                assert_eq!(setting.kind, Kind::Bool, "{} is not a switch", setting.key);
            }
        }
        let profile = ENERGY
            .iter()
            .find(|setting| setting.key == "profile")
            .expect("the Energy page has no trigger profile");
        assert!(matches!(profile.kind, Kind::Choice(_)));
        assert!(crate::energy::PROFILES.contains(&profile.default));
        // The page's buttons are distinct and name the two verbs the element
        // takes, so one cannot shadow the other.
        assert_eq!(
            ENERGY_ACTIONS
                .iter()
                .map(|command| command.id)
                .collect::<Vec<_>>(),
            vec!["apply", "restore"]
        );
    }
}
