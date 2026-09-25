-- hypr-osd ↔ Hyprland integration
-- ---------------------------------------------------------------------------
-- Installed by scripts/install.sh and loaded from ~/.config/hypr/hyprland.lua
-- with a single line:
--
--     require("osd")
--
-- This file is the source of truth for the bar, the volume keys, the autostart
-- and the layer rule on Hyprland (>= 0.55 Lua config). Re-run the installer to
-- regenerate it after editing this template in the repository.
--
-- Nothing here is required for the elements to work: they are ordinary
-- binaries, and `hypr-osd-bar` in a terminal does exactly what the autostart
-- below does. This file is only how the compositor gets to them.

-- Absolute paths of the installed binaries. The installer substitutes the real
-- paths; `$HOME` style expansion happens in sh, not here, so they stay literal.
local bar = "@BAR_OSD_BIN@"
local island = "@ISLAND_OSD_BIN@"
local stats = "@STATS_OSD_BIN@"
local volume_osd = "@VOLUME_OSD_BIN@"
local media_osd = "@MEDIA_OSD_BIN@"
local session_osd = "@SESSION_OSD_BIN@"
local switcher_osd = "@SWITCHER_OSD_BIN@"
local overview_osd = "@OVERVIEW_OSD_BIN@"

-- Start the daemons at login. The bar is visible from that moment (it is the
-- bar); the cards show nothing until they are asked to. They all sit in the
-- background (one small GTK process each) so the first key press, and the first
-- track that starts playing, have a card ready instead of paying for GTK
-- start-up. The switcher is the one that really wants this: its card has to be
-- up before you let go of Alt. Set to false to start them lazily - the first key
-- press still works, it just takes a moment longer, and a track that starts
-- before the daemon is up is missed.
local AUTOSTART = true

if AUTOSTART then
    -- `hyprland.start` runs on the first config parse of a session; a
    -- `hyprctl reload` later re-parses the config but does not re-run it
    -- (that is what makes it "once", like `exec-once`).
    hl.on("hyprland.start", function()
        hl.exec_cmd(bar)
        -- The island popup is a program of its own because a GTK widget cannot
        -- paint outside its own window: the panel that unfolds from the bar's
        -- clock is a second layer-shell surface. It shows nothing until the
        -- clock is hovered, which is why it can be running all the time.
        hl.exec_cmd(island)
        -- The same for the system popup, which unfolds from the bar's status
        -- pill and hangs under the bar's right end.
        hl.exec_cmd(stats)
        hl.exec_cmd(volume_osd)
        hl.exec_cmd(media_osd)
        hl.exec_cmd(session_osd)
        hl.exec_cmd(switcher_osd)
        hl.exec_cmd(overview_osd)

        -- hypridle locks on idle and before sleep (see the lock screen section
        -- below). It is optional and independent of the OSDs, so it is started
        -- only when it is installed - a missing binary is not an error here, and
        -- `sh -c` is what makes the test and the start one command (Hyprland's
        -- exec does not report a failure anywhere the user would see it).
        hl.exec_cmd("sh -c 'command -v hypridle >/dev/null 2>&1 && hypridle'")
    end)
end


-------------------
---- THE BAR ------
-------------------

-- The bar is not an OSD: it owns a layer-shell surface across the top of every
-- screen (one per monitor, kept in step with the compositor's monitor list, so
-- plugging a screen in grows a bar onto it and unplugging one takes that bar
-- away), reserves its strip with the compositor (so tiled windows start below
-- it), and is up for the whole session. Everything on it - the workspaces, the
-- focused window's title, what is playing, the tray, the system readings, the
-- link/sound/battery pill and the clock - is read and drawn by the bar itself:
-- no helper scripts, and nothing to configure here beyond starting it. Set
-- `output` in ~/.config/hypr-osd/bar.conf to put it on one screen only.
--
-- It deliberately has no keybinding. Its verbs exist for scripting and for
-- testing, and reach the running bar over D-Bus:
--
--     hypr-osd-bar hide | show | toggle | refresh | status
--
-- `status` prints what every pill currently reads and which screens the bars are
-- on, which is the fastest way to answer "why does the volume pill say 0 %" and
-- "why is there no bar on that monitor".
--
-- Two of its pills are *handles* for the popups: the clock unfolds the island
-- (hover), and the status pill unfolds the system popup. Both panels decide for
-- themselves when to close, which is the only way it can work - the pointer
-- leaving the pill for the panel is leaving the bar's surface, which the bar
-- never sees. See the island and the system popup sections below.
--
-- The bar is also the tray host: it owns org.kde.StatusNotifierWatcher on the
-- session bus, which is what makes application indicators visible at all. That
-- is one session-wide object with one set of icons, so exactly one of the bars
-- carries it: the screen named by `output`, else the one that had the focus when
-- the bar started. An indicator that started *before* the bar registered will
-- not appear until it is restarted (indicators register once, when they start).
--
-- Hovering the clock unfolds the island popup, and hovering the status pill
-- unfolds the system popup (see the popups section below). Nothing else about
-- the bar needs a key or a rule.


---------------------
---- KEYBINDINGS ----
---------------------

-- The OSD owns the volume *step*, not only the display: it runs wpctl itself and
-- then shows what the sink actually reports. That means these three binds must
-- be the only ones on these keys - if hyprland.lua still has its own
-- `wpctl set-volume ...` binds, the volume moves twice per press. The installer
-- reports them, and `--take-over-keys` comments them out for you.
--
-- `locked` so they keep working on the lock screen - the volume still moves
-- there, though no card is drawn: a session lock shows nothing but the locker's
-- own surface, which is the point of it. `repeating` so holding the key keeps
-- stepping - the same options the stock binds use.
hl.bind("XF86AudioRaiseVolume", hl.dsp.exec_cmd(volume_osd .. " up"), {
    locked = true,
    repeating = true,
})

hl.bind("XF86AudioLowerVolume", hl.dsp.exec_cmd(volume_osd .. " down"), {
    locked = true,
    repeating = true,
})

hl.bind("XF86AudioMute", hl.dsp.exec_cmd(volume_osd .. " toggle"), {
    locked = true,
    repeating = true,
})

-- The microphone key (XF86AudioMicMute) is deliberately left alone: the volume
-- element is about the sink. Its `wpctl set-mute @DEFAULT_AUDIO_SOURCE@ toggle`
-- bind in hyprland.lua keeps working, or can be pointed at
-- `hypr-osd-volume set-mic-toggle` once there is an element for it.


------------------------------------
---- THE POPUPS (no binds) ---------
------------------------------------

-- Two of the bar's pills are *handles*: the clock unfolds the island (what is
-- playing, what is waiting, the calendar) and the status pill unfolds the system
-- popup (the readings, the pending updates, and the switches: bluetooth, the
-- link, the sound, the power profile, presentation mode, brightness and the
-- keyboard layout).
--
-- Neither is a window that opens: each is a second layer-shell surface of its
-- own, hanging below the bar, which is why each is its own program (a GTK widget
-- cannot paint outside the window it lives in). The bar asks for the panel when
-- the pointer arrives on the pill; from then on the panel watches the pointer
-- itself, because leaving the pill means moving onto the panel - somewhere the
-- bar cannot see.
--
-- The verbs are for scripting and for testing; both reach the running daemon
-- over D-Bus:
--
--     hypr-osd-island open [connector] | close | toggle | show | status
--     hypr-osd-stats  open [connector] [x y w h] | close | toggle | show | wifi | status
--
-- `status` is the fastest way to answer "why does the popup say that", and it
-- prints its answer without ever drawing a card - including where the two hover
-- zones ended up, which is how "the panel does not open" gets diagnosed. `wifi`
-- switches the wireless radio, which is what the status pill's right click runs:
-- the switch has to read `rfkill` before it can flip it, and that reading lives
-- in the element.
--
-- `open` carries two things the panel cannot work out for itself. The connector
-- says which of the bars the pointer came from - there is one per monitor, and
-- each is the same program in a different surface - because the popup has to be
-- drawn on that screen and measured against it. The four numbers are the pill's
-- rectangle inside that bar's card, so the hot zone is exactly the pill and never
-- reaches the tray beside it. Typed by hand without either, the pointer is
-- watched against a strip at the bar's right end on the focused monitor.
--
-- The system popup also holds the presentation switch, which is a real
-- `systemd-inhibit` child: it is released when the element exits, so a stale
-- block cannot outlive the session's UI.


------------------------------------
---- THE MEDIA CARD (no binds) -----
------------------------------------

-- The media card is event-driven: the daemon follows `playerctl metadata
-- --follow` and shows the card when a new track starts playing, whichever way it
-- started. There is nothing to bind.
--
-- It is still useful to have a key for "what is playing?" - this shows the card
-- for the current track on demand. SUPER + SHIFT + M is a suggestion; pick
-- whatever is free in hyprland.lua (SUPER + M is a workspace bind by default):
--
-- hl.bind("SUPER + SHIFT + M", hl.dsp.exec_cmd(media_osd .. " show"), {
--     description = "Media: show what is playing",
-- })


------------------------------------
---- THE SESSION CARD ---------------
------------------------------------

-- Lock, suspend, log out, reboot and shut down, in a card. This *is* the
-- desktop's session menu now: the bar it used to be duplicated in is gone, so
-- the card is the only place these five actions live (the five buttons of the
-- tile it was modelled on, `~/.config/waybar/scripts/popup.py`, with the same
-- glyphs and the same destructive-actions-ask-twice rule).
--
-- `toggle` rather than `show`: pressing the key again takes the card away, which
-- is the only way to dismiss it without choosing something (a layer surface
-- never gets the keyboard, so Escape cannot reach it - the card says so in its
-- own hint line).
hl.bind("SUPER + SHIFT + L", hl.dsp.exec_cmd(session_osd .. " toggle"), {
    locked = true,
})

-- The hardware power key. Whether Hyprland sees it at all depends on
-- systemd-logind: with `HandlePowerKey=` set to anything but `ignore`, logind
-- acts on the key itself (this machine suspends) and may consume it before any
-- compositor binding runs. `scripts/install.sh` reports the current setting and
-- the two lines that hand the key over.
--
-- With hypridle running (see the lock screen section), leaving logind alone is
-- not the wrong answer either: hypridle's `before_sleep_cmd` locks the screen
-- first, so the key then suspends an already locked machine instead of showing
-- this card.
hl.bind("XF86PowerOff", hl.dsp.exec_cmd(session_osd .. " toggle"), {
    locked = true,
})


------------------------------------
---- THE WINDOW SWITCHER -----------
------------------------------------

-- Alt-Tab: every window you have open, in the order you last used them, with
-- its application icon and its title, in a grid in the middle of the screen.
--
-- This is the one element that takes the keyboard, and it has to be: a switch
-- ends when you let go of Alt, and the only way to see that release is to own
-- the keyboard for as long as the card is up. It is a held gesture, so nothing
-- else is affected - and the card is gone the moment you let go (or press
-- Enter, or Escape to walk away without switching).
--
-- No `repeating`: holding the key down *is* the walk, and those repeats arrive
-- on the card's own keyboard rather than through this bind - which is also why
-- the first press is the only one Hyprland ever sees. No `locked` either: a
-- window switcher over the lock screen is exactly the wrong thing.
hl.bind("ALT + TAB", hl.dsp.exec_cmd(switcher_osd .. " next"))
hl.bind("ALT + SHIFT + TAB", hl.dsp.exec_cmd(switcher_osd .. " prev"))


------------------------------------
---- THE WORKSPACE OVERVIEW --------
------------------------------------

-- SUPER + SHIFT + TAB: one tap, and the whole desktop is on the screen as a
-- picture of itself - every workspace as a chip along the top, every window as
-- a tile in its own shape, all of them scaled to fit at once. Click a tile (or
-- press Enter) to go there; the digit keys switch workspace; Escape leaves.
--
-- A tap rather than a hold, and the key is a *toggle*: the same gesture takes
-- the card away again. Like the switcher, this card takes the keyboard while it
-- is up - Escape and the arrows have to be the card's own keys, or they would
-- be fighting whatever is behind it. It is the second element that does, and
-- both of them are cards you opened on purpose.
--
-- No `locked`: an overview of the desktop is nothing to show over a lock screen.
hl.bind("SUPER + SHIFT + TAB", hl.dsp.exec_cmd(overview_osd .. " toggle"))


--------------------------------
---- THE LOCK SCREEN -----------
--------------------------------

-- hyprlock draws the lock screen. It is a program of its own with a config of
-- its own (~/.config/hypr/hyprlock.conf, installed from this repository's
-- configs/); Hyprland only has to start it. Everything that locks - the session
-- card's Lock row, its Suspend row, the idle timers, and this key - leads to the
-- same screen, because they all end up running the same binary.
--
-- SUPER + SHIFT + L opens the session card, so bare SUPER + L gets the lock.
-- `pidof` first: pressing the key on an already locked screen should do nothing,
-- not start a second hyprlock (it would refuse the lock and log an error).
hl.bind("SUPER + L", hl.dsp.exec_cmd("sh -c 'pidof hyprlock >/dev/null || hyprlock'"), {
    locked = true,
})

-- Idle locking is hypridle's job, not Hyprland's: `~/.config/hypr/hypridle.conf`
-- (installed from configs/) locks after 5 minutes, blanks the panel at 10 and
-- suspends at 30. hypridle is started in the autostart block above when it is
-- installed, and the config is only ever read if it is - so skip
--     sudo pacman -S hypridle
-- if a lock screen only when you ask for one is what you want.


-------------------
---- LAYER RULE ----
-------------------

-- The bar and every card are drawn on layer-shell surfaces, which are not
-- windows: no window rules apply to them, and they need none. Hyprland places
-- them (the bar across the top, a card at the bottom edge or in the middle, on
-- the focused monitor, above fullscreen windows), and none of them ever takes
-- keyboard focus - with the two deliberate exceptions noted above.
--
-- The one thing worth configuring is what happens to the *transparent* frame
-- around a card: it is part of the surface, so without this rule it would
-- swallow clicks on whatever is behind it. `ignore_alpha` makes pixels below
-- 20 % opacity click-through, which leaves the card itself as the only part of an
-- OSD you can actually hit - the volume slider and the media card's skip buttons
-- need that, and the shadow around a card does not. One rule covers every
-- element, because they share the `hypr-osd` namespace prefix.
hl.layer_rule({
    name  = "hypr-osd",
    match = { namespace = "^hypr-osd" },
    ignore_alpha = 0.2,
})
