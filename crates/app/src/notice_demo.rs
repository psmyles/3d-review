//! **TEMPORARY** — manual trigger for every notification kind, so the notice
//! column can be checked by hand without having to provoke a real load, export
//! or device fault.
//!
//! Delete this whole file when the check is done. Removing it takes exactly
//! three other edits, all marked `notice_demo`: the `mod` line in `main.rs`, the
//! two `demo_*_step` fields beside it, and the dispatch arm in `shortcuts.rs`.
//!
//! These bindings are deliberately **not** in `ui`'s help card. Everything in
//! `shortcuts.rs` is normally mirrored there, but the help card is the
//! user-facing shortcut list and these are going away.
//!
//! | Chord | What it posts |
//! |---|---|
//! | `Primary+Shift+I` | an info notice (auto-dismisses) |
//! | `Primary+Shift+S` | a success notice (auto-dismisses) |
//! | `Primary+Shift+W` | a warning (sticky) |
//! | `Primary+Shift+E` | an error (sticky) |
//! | `Primary+Shift+R` | a grouped report with more lines than it will show |
//! | `Primary+Shift+M` | a mode notice — bare single line; press repeatedly to watch it rewrite in place |
//! | `Primary+Shift+L` | a very long title, to watch it truncate |
//! | `Primary+Shift+P` | the next step of a fake background job |
//!
//! `Primary+Shift+P` is press-driven rather than timed on purpose: it walks the
//! progress card through the exact sequence a real load produces — named, then a
//! stage line, then a bar, then a stage with no bar, then finished plus its
//! success notice — one step per press, which is the sequence that made the
//! column grow a card's height per load.

use review_ui::NoticeKind;

use crate::App;

/// How many presses one full fake job takes.
const JOB_STEPS: usize = 6;

impl App {
    /// Handle a `Primary+Shift+<key>` demo chord. Returns whether `key` was one.
    pub(crate) fn handle_notice_demo_key(&mut self, key: &str) -> bool {
        match key {
            "i" => self
                .notifications
                .info("Nothing is wrong, but you should know"),
            "s" => self
                .notifications
                .success("Loaded cpg_pedestal_pebbles.fbx"),
            "w" => self
                .notifications
                .warning("Texture auto-reload unavailable (file watcher failed)"),
            "e" => self
                .notifications
                .error("Couldn't load pedestal.fbx: unexpected end of file"),
            "r" => self.notifications.report(
                NoticeKind::Success,
                "Exported AN_ZombiedogLocomotion.fbx (5139 triangles)",
                // More lines than the card shows, so the `+ n more` summary is
                // on screen too.
                (1..=9)
                    .map(|n| {
                        format!(
                            "LOD 0: 'ZombieDog_Part_{n:02}' was written as triangles — \
                             the stack rebuilt its geometry."
                        )
                    })
                    .collect(),
            ),
            "m" => {
                // Rewrites one card in place however fast it is pressed.
                self.notifications
                    .mode(format!("Mode {}", self.demo_mode_step));
                self.demo_mode_step = self.demo_mode_step.wrapping_add(1);
            }
            "l" => self.notifications.success(format!(
                "Loaded {}.fbx",
                "a_very_long_asset_name_that_will_not_fit_".repeat(4)
            )),
            "p" => self.step_demo_job(),
            _ => return false,
        }
        self.redraw.requested = true;
        true
    }

    /// Advance the fake background job one step per press, wrapping round.
    fn step_demo_job(&mut self) {
        let step = self.demo_job_step % JOB_STEPS;
        self.demo_job_step = self.demo_job_step.wrapping_add(1);
        match step {
            0 => self
                .notifications
                .begin_activity("Loading cpg_pedestal_pebbles.fbx…"),
            1 => self
                .notifications
                .update_activity("Reading… 18%", Some(0.18)),
            // A stage that can't report a fraction: the bar goes away and the
            // card must not resize around it.
            2 => self
                .notifications
                .update_activity("Building geometry…", None),
            3 => self
                .notifications
                .update_activity("Reading… 91%", Some(0.91)),
            4 => self
                .notifications
                .update_activity("Measuring draw groups…", Some(1.0)),
            _ => {
                self.notifications.end_activity();
                self.notifications
                    .success("Loaded cpg_pedestal_pebbles.fbx");
            }
        }
    }
}
