//! `hypr-osd-energy` - element #9: the energy-saving daemon.
//!
//! The settings window's **System** tab is where the policy is edited (which
//! power profile means "save energy", and which of the compositor's effects are
//! worth dropping while it is set). This element is what *enforces* it: it reads
//! `~/.config/hypr-osd/energy.conf`, watches `powerprofilesctl`, and hands the
//! compositor a small generated document whenever the values it wants are not the
//! values the compositor has.
//!
//! It is an element like every other - one GTK application, one binary, verbs on
//! the command line - but it is the collection's only **headless** one: it never
//! shows a card, because its business is the compositor's options rather than a
//! window. Being an element is still what makes it work: it is single-instance by
//! D-Bus name and autostarted, so the settings window's two buttons (and a
//! script) can talk to the one running copy.
//!
//! Two things it is careful about:
//!
//! * **It remembers before it overrides.** The first time a saving profile takes
//!   hold, every effect key is read off the *running* compositor and written to
//!   `$XDG_RUNTIME_DIR/hypr-osd-energy.state`. Leaving the saving state puts those
//!   values back, so a machine whose own `hyprland.lua` says something this
//!   collection does not know about gets it back exactly (see [`Baseline`]).
//! * **It only writes when it disagrees.** Every poll compares the values it
//!   wants with the ones the compositor has; the usual answer is "the same", and
//!   then nothing is run at all. That is also what makes the daemon self-healing:
//!   a settings change that restored an effect under a saving profile is corrected
//!   on the next poll.
//!
//! Verbs (`hypr-osd-energy <verb>`):
//!
//! ```text
//!   status               the profile, the policy, and every effect key's value
//!   apply | save         drop the effects now, and keep them dropped until
//!                        `restore` (the profile no longer matters)
//!   restore              put the remembered values back now
//!   refresh | tick       re-read the policy and decide again
//!   (no verb)            start the daemon and watch the profile
//! ```
//!
//! `apply`/`save` and `restore` are what the System tab's two buttons run.

use std::cell::{Cell, OnceCell, RefCell};
use std::fs;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use hypr_osd_core::energy::{self, Baseline, Policy};
use hypr_osd_core::{css, hyprctl, run, Config, Content, Opts, Osd};

/// D-Bus application id - and therefore the single-instance key: the settings
/// window's buttons reach the running daemon instead of starting a second one.
const APP_ID: &str = "com.schells2.osd.energy";
/// Layer-shell namespace, the collection's shared prefix.
const NAMESPACE: &str = "hypr-osd";
/// Element name: `~/.config/hypr-osd/energy.conf`.
const ELEMENT: &str = "energy";

/// The daemon's whole state: the policy it is enforcing, the values it
/// remembered before overriding, what it last did (which is what `status`
/// and the buttons' answers show), and whether saving was asked for by hand.
struct Daemon {
    policy: RefCell<Policy>,
    baseline: RefCell<Baseline>,
    applied: RefCell<String>,
    /// Set by `apply` and cleared by `restore`: a manual save that outlasts the
    /// poll that would otherwise undo it. Deliberately *not* persisted - a
    /// restart drops the override with it, and the profile decides again.
    forced: Cell<bool>,
}

impl Daemon {
    fn new() -> Rc<Self> {
        Rc::new(Daemon {
            policy: RefCell::new(Policy::load(&Config::load(ELEMENT))),
            // A daemon restarted mid-save finds the values it captured before, so
            // it does not mistake its own overrides for the machine's own state.
            baseline: RefCell::new(Baseline::load(&energy::state_path()).unwrap_or_default()),
            applied: RefCell::new("nothing yet".to_string()),
            forced: Cell::new(false),
        })
    }

    /// One poll: read the profile, decide, and act only if the compositor
    /// disagrees.
    fn tick(&self) {
        let active = energy::active_profile();
        let saving = self.forced.get() || energy::saving(&self.policy.borrow(), active.as_deref());
        self.set_state(saving);
    }

    /// Bring the compositor to the state that matches `saving`.
    fn set_state(&self, saving: bool) {
        let was_saving = self.baseline.borrow().saving;
        self.remember(saving);

        // A machine that has never entered the saving state keeps its own values,
        // untouched. This is what stops the daemon from "restoring" a catalogue
        // default over a hand-written config on a plain boot: with no remembered
        // baseline there is nothing to put back, and the only honest move is to
        // not write at all.
        if !saving && !was_saving {
            *self.applied.borrow_mut() = "leaving the effects alone".to_string();
            return;
        }

        // The values wanted right now: the generated document is complete, so a
        // reader can see the whole decision in one place.
        let entries = {
            let policy = self.policy.borrow();
            let baseline = self.baseline.borrow();
            energy::desired(&policy, &baseline, saving)
        };

        let live = energy::live_values();
        if energy::agrees(&entries, &live) {
            // The usual answer, and the one that keeps a poll free: nothing is
            // written and no process is started.
            *self.applied.borrow_mut() = if saving {
                "already saving".to_string()
            } else {
                "already at the machine's own values".to_string()
            };
            return;
        }

        self.write(&entries, saving);
    }

    /// Capture the machine's own values on the way *into* the saving state, and
    /// mark the state on the way out. The values themselves are kept either way,
    /// because they are what "restore" means.
    fn remember(&self, saving: bool) {
        let mut baseline = self.baseline.borrow_mut();
        if saving && !baseline.saving {
            // Read the compositor *before* anything is overwritten - a moment
            // later and the baseline would be our own override.
            *baseline = Baseline::capture();
            if let Err(error) = baseline.save(&energy::state_path()) {
                eprintln!("hypr-osd-energy: cannot remember the current values: {error}");
            }
        } else if !saving && baseline.saving {
            baseline.saving = false;
            if let Err(error) = baseline.save(&energy::state_path()) {
                eprintln!("hypr-osd-energy: cannot update the remembered state: {error}");
            }
        }
    }

    /// Write the document and hand it to the compositor.
    ///
    /// `hyprctl eval` rather than a socket write, for the same reason the settings
    /// window uses it: the compositor's complaint about a document it will not
    /// take is the only thing that makes a failure fixable, and it names the
    /// option it did not like.
    fn write(&self, entries: &[energy::Entry], saving: bool) {
        let document = energy::document(entries);
        let path = energy::lua_path();
        let outcome = (|| -> Result<String, String> {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
            }
            fs::write(&path, document)
                .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
            hyprctl::eval(&format!("dofile(\"{}\")", path.display()))
        })();

        *self.applied.borrow_mut() = match outcome {
            Ok(said) => {
                let state = if saving { "saving" } else { "restored" };
                if said.is_empty() {
                    state.to_string()
                } else {
                    format!("{state}: {said}")
                }
            }
            Err(why) => {
                eprintln!("hypr-osd-energy: Hyprland refused the effects: {why}");
                format!("failed: {why}")
            }
        };
    }

    /// Re-read the policy file, and the remembered state, from disk.
    fn refresh(&self) {
        *self.policy.borrow_mut() = Policy::load(&Config::load(ELEMENT));
        if let Some(baseline) = Baseline::load(&energy::state_path()) {
            *self.baseline.borrow_mut() = baseline;
        }
    }

    /// The `status` verb: everything a person needs to answer "why is my blur
    /// still on" without opening a window.
    fn status(&self) -> String {
        let policy = self.policy.borrow();
        let baseline = self.baseline.borrow();
        let active = energy::active_profile();
        let by_profile = energy::saving(&policy, active.as_deref());
        let saving = self.forced.get() || by_profile;
        let state = if !saving {
            "normal".to_string()
        } else if by_profile {
            "saving".to_string()
        } else {
            "saving (asked for by hand)".to_string()
        };
        let live = energy::live_values();
        let entries = energy::desired(&policy, &baseline, saving);

        let mut out = format!(
            "profile    {}\npolicy     {} (trigger {}, every {} ms)\nstate      {}\nremembered {} values\napplied    {}\n",
            active.as_deref().unwrap_or("(none)"),
            if policy.enabled { "on" } else { "off" },
            policy.profile,
            policy.poll.as_millis(),
            state,
            baseline.remembered(),
            self.applied.borrow(),
        );
        for (key, _, want) in &entries {
            let have = live
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.as_str())
                .unwrap_or("?");
            out.push_str(&format!("  {key} = {have}  (want {want})\n"));
        }
        out.trim_end().to_string()
    }
}

fn main() -> glib::ExitCode {
    let opts = Opts {
        app_id: APP_ID.to_string(),
        css: css::stylesheet(include_str!("energy.css")),
        namespace: NAMESPACE.to_string(),
        ..Opts::default()
    };

    let state_cell: Rc<OnceCell<Rc<Daemon>>> = Rc::new(OnceCell::new());

    let build = {
        let state_cell = state_cell.clone();
        Box::new(move |_osd: &Rc<Osd>| {
            let daemon = Daemon::new();
            // The first decision at once, so a machine already in a saving
            // profile does not spend one poll interval with its effects on.
            daemon.tick();

            // Then on every poll. The interval is read at start-up on purpose: a
            // `poll_ms` change is what the System tab's Restart chip is for.
            let interval = daemon.policy.borrow().poll;
            {
                let daemon = daemon.clone();
                glib::timeout_add_local(interval, move || {
                    daemon.tick();
                    glib::ControlFlow::Continue
                });
            }

            state_cell.set(daemon).ok();

            // Nothing is ever shown. The card exists because `run` gives every
            // element one surface; this element's work is in the compositor's
            // options, so it is created and left unmapped.
            let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
            Content::Single(root.upcast())
        })
    };

    let handle = {
        let state_cell = state_cell.clone();
        Rc::new(
            move |_osd: &Rc<Osd>, args: &[String]| -> Result<String, String> {
                let daemon = state_cell
                    .get()
                    .ok_or("the energy daemon has not finished starting up")?;
                match args.first().map(String::as_str) {
                    // No verb: started by the autostart, so just watch.
                    None => Ok(String::new()),
                    Some("tick") | Some("refresh") => {
                        daemon.refresh();
                        daemon.tick();
                        Ok(daemon.applied.borrow().clone())
                    }
                    Some("apply") | Some("save") => {
                        daemon.forced.set(true);
                        daemon.tick();
                        Ok(daemon.applied.borrow().clone())
                    }
                    Some("restore") => {
                        daemon.forced.set(false);
                        daemon.tick();
                        Ok(daemon.applied.borrow().clone())
                    }
                    Some("status") => Ok(daemon.status()),
                    Some(other) => Err(format!(
                        "unknown verb `{other}` (status | apply | save | restore | refresh | tick)"
                    )),
                }
            },
        )
    };

    run(opts, build, handle)
}
