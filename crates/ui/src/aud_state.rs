//! UI state for the Aud workspace: the audit profile the user edits, the last
//! report `app` handed over, and how the Issues list and the viewport present
//! it.
//!
//! Ownership follows Opt's convention. `UiState` holds the profile, the chrome
//! edits it in place, and bumping [`AudUiState::profile_revision`] is what tells
//! `app` to re-run the audit and what the undo snapshot compares. Only the
//! actions that reach outside the app — the file dialogs behind profile and
//! report IO — travel as [`AuditIntent`]s.

use std::collections::BTreeSet;
use std::sync::Arc;

use review_audit::{AuditProfile, AuditReport, Category, DiagnosticView, RuleId};

/// What the audit is focused on: the Issues-list row the user picked, and so
/// what the viewport highlights and the Inspector explains.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditFocus {
    /// A whole check: every object that fails it.
    Rule(RuleId),
    /// One object's share of a check.
    Offender(RuleId, usize),
    /// Every finding of one object (the By object grouping's object rows).
    Object(usize),
}

impl AuditFocus {
    /// The rule in focus, if the focus is on one.
    pub fn rule(self) -> Option<RuleId> {
        match self {
            AuditFocus::Rule(rule) | AuditFocus::Offender(rule, _) => Some(rule),
            AuditFocus::Object(_) => None,
        }
    }

    /// The object in focus, if the focus is narrowed to one.
    pub fn node(self) -> Option<usize> {
        match self {
            AuditFocus::Offender(_, node) | AuditFocus::Object(node) => Some(node),
            AuditFocus::Rule(_) => None,
        }
    }
}

/// How the Issues list is grouped.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AudGrouping {
    /// Category → check → the objects that fail it.
    #[default]
    ByCheck,
    /// Object → its findings, with scene-wide findings in their own group.
    ByObject,
}

/// One collapsible group of the Issues list, remembered across frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum IssueGroup {
    Category(Category),
    Rule(RuleId),
    /// An object in the By object grouping; `None` is the scene-wide group.
    Object(Option<usize>),
}

/// Everything the Aud workspace keeps in the UI.
#[derive(Debug, Clone)]
pub struct AudUiState {
    /// The active profile. Behind an `Arc` so `app` can hand it to the audit
    /// worker and the undo snapshot can hold it without cloning.
    pub profile: Arc<AuditProfile>,
    /// Bumped on every edit of `profile`; `app` re-runs the audit when it moves
    /// and the undo snapshot compares it.
    pub profile_revision: u64,
    /// The newest completed report for the model on screen, from `app`.
    pub report: Option<Arc<AuditReport>>,
    /// A run is in flight.
    pub running: bool,
    /// What the Issues list has selected, and the viewport highlights.
    pub focus: Option<AuditFocus>,
    /// Whether the Inspector is showing the profile editor (the Issues tab's
    /// Profile row) rather than the focus.
    pub profile_open: bool,
    /// The diagnostic view the viewport shows. Selecting a finding sets it to
    /// that finding's related view.
    pub view: DiagnosticView,
    pub grouping: AudGrouping,
    /// Whether checks that passed (or did not run) are listed too.
    pub show_passed: bool,
    /// Groups the user has flipped from their default: categories and objects
    /// start open, a check's list of objects starts closed.
    pub toggled: BTreeSet<IssueGroup>,
}

impl Default for AudUiState {
    fn default() -> Self {
        Self {
            profile: Arc::new(AuditProfile::default()),
            profile_revision: 0,
            report: None,
            running: false,
            focus: None,
            profile_open: false,
            view: DiagnosticView::Issues,
            grouping: AudGrouping::default(),
            show_passed: false,
            toggled: BTreeSet::new(),
        }
    }
}

impl AudUiState {
    /// Edit the profile through `edit` and bump the revision. Every profile edit
    /// goes through here so no caller can forget the bump.
    pub fn edit_profile(&mut self, edit: impl FnOnce(&mut AuditProfile)) {
        edit(Arc::make_mut(&mut self.profile));
        self.profile_revision = self.profile_revision.saturating_add(1);
    }

    /// Replace the profile wholesale (a loaded file, a built-in, an undo).
    pub fn set_profile(&mut self, profile: Arc<AuditProfile>) {
        self.profile = profile;
        self.profile_revision = self.profile_revision.saturating_add(1);
    }

    /// Drop everything that describes the outgoing model: its report and the
    /// focus into it. The profile and the list's presentation stay.
    pub fn reset_for_new_model(&mut self) {
        self.report = None;
        self.running = false;
        self.focus = None;
        self.view = DiagnosticView::Issues;
    }

    /// Whether a group is open.
    pub fn is_open(&self, group: IssueGroup) -> bool {
        let open_by_default = !matches!(group, IssueGroup::Rule(_));
        open_by_default != self.toggled.contains(&group)
    }

    pub fn toggle(&mut self, group: IssueGroup) {
        if !self.toggled.remove(&group) {
            self.toggled.insert(group);
        }
    }
}

/// An Aud action `app` must carry out: the file dialogs behind the profile and
/// the report. Everything else is a plain [`AudUiState`] field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditIntent {
    SaveProfile,
    LoadProfile,
    SaveReport,
    /// The chrome put the summary on the clipboard; `app` says so.
    SummaryCopied,
}
