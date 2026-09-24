-- hypr-osd ↔ Hyprland integration
-- ---------------------------------------------------------------------------
-- Installed by scripts/install.sh and loaded from ~/.config/hypr/hyprland.lua
-- with a single line:
--
--     require("osd")
--
-- This file is the source of truth for the volume keys, the autostart and the
-- layer rule on Hyprland (>= 0.55 Lua config). Re-run the installer to
-- regenerate it after editing this template in the repository.
--
-- Nothing here is required for the elements to work: they are ordinary
-- binaries, and `hypr-osd-volume up` in a terminal does exactly what the
-- keybinding below does. This file is only how the compositor gets to them.

-- Absolute paths of the installed binaries. The installer substitutes the real
-- paths; `$HOME` style expansion happens in sh, not here, so they stay literal.
local volume_osd = "@VOLUME_OSD_BIN@"
local media_osd = "@MEDIA_OSD_BIN@"
local session_osd = "@SESSION_OSD_BIN@"
local switcher_osd = "@SWITCHER_OSD_BIN@"

-- Start the daemons at login. Nothing is displayed: they sit in the background
-- (one small GTK process each) so the first key press, and the first track that
-- starts playing, have a card ready instead of paying for GTK start-up. The
-- switcher is the one that really wants this: its card has to be up before you
-- let go of Alt. Set to false to start them lazily - the first key press still
-- works, it just takes a moment longer, and a track that starts before the
-- daemon is up is missed.
local AUTOSTART = true

if AUTOSTART then
    -- `hyprland.start` runs on the first config parse of a session; a
    -- `hyprctl reload` later re-parses the config but does not re-run it
    -- (that is what makes it "once", like `exec-once`).
    hl.on("hyprland.start", function()
        hl.exec_cmd(volume_osd)
        hl.exec_cmd(media_osd)
        hl.exec_cmd(session_osd)
        hl.exec_cmd(switcher_osd)

        -- hypridle locks on idle and before sleep (see the lock screen section
        -- below). It is optional and independent of the OSDs, so it is started
        -- only when it is installed - a missing binary is not an error here, and
        -- `sh -c` is what makes the test and the start one command (Hyprland's
        -- exec does not report a failure anywhere the user would see it).
        hl.exec_cmd("sh -c 'command -v hypridle >/dev/null 2>&1 && hypridle'")
    end)
end


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

-- Lock, suspend, log out, reboot and shut down, in a card - the same menu the
-- bar's popup already offers (see the waybar popup at SUPER + M, or whatever
-- else you have bound to hyprshutdown), but drawn where you are looking.
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

-- The card is drawn on a layer-shell surface, which is not a window: no window
-- rules apply to it, and it needs none. Hyprland places it (bottom edge, on the
-- focused monitor, above fullscreen windows), and it never takes keyboard focus.
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
