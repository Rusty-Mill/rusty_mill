//! The service: resolves actors from roles, applies domain operations and
//! commits them through a [`Store`], retrying on revision conflicts.

use std::cell::Cell;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError, RwLock, RwLockReadGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::{
    Actor, BoardError, Capability, Card, Column, DEFAULT_PROJECT, DONE_WINDOW_DAYS, Dates, Edit,
    Event, EventKind, ExecutionLink, MoveRules, Plan, Shelf, import_id, text_hash,
};
use crate::inbox::{Acted, Action, DryRun, Escalation, Escalations, InboxError, Outcome};
use crate::metrics::{Metrics, metrics};
use crate::milestones::{MemMilestones, Milestone, MilestoneEdit, MilestoneStore};
use crate::projects::Projects;
use crate::roadmap::{Roadmap, roadmap};
use crate::store::Store;
use crate::trace::{self, MilestoneView, Trace};
use crate::workflow::Workflow;

/// Attempts per operation when another writer wins the revision race.
const RETRIES: usize = 3;

/// The actor named on the events that number cards.
const NUMBERER: &str = "board";

/// Role name -> capabilities. The board enforces roles; defining them is
/// the caller's job (orch), so [`Roles::default`] is only a starting set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Roles(pub BTreeMap<String, BTreeSet<Capability>>);

/// The capability, besides `Triage`, that re-files a card in `column`: closing work is what
/// files a Done card, so `Close` re-files one.
fn assign_capability(column: &Column) -> Capability {
    if *column == Column::DONE {
        Capability::Close
    } else {
        Capability::Project
    }
}

impl Default for Roles {
    fn default() -> Self {
        use Capability::*;
        let role = |caps: &[Capability]| caps.iter().copied().collect();
        Self(BTreeMap::from([
            (
                "human".into(),
                role(&[
                    Triage, Plan, Architect, Build, Test, Review, Merge, Close, Project, Admin,
                ]),
            ),
            ("builder".into(), role(&[Build, Close, Project])),
            ("planner".into(), role(&[Plan])),
            ("architect".into(), role(&[Architect])),
            ("tester".into(), role(&[Test])),
            ("qa".into(), role(&[Test])),
            // Merging is the human's gate, so the reviewer passes work on but never merges.
            ("reviewer".into(), role(&[Review])),
        ]))
    }
}

impl Roles {
    /// The actor `name` acting as `role`.
    ///
    /// # Errors
    /// If the role is unknown or the name blank.
    pub fn actor(&self, name: &str, role: &str) -> Result<Actor, BoardError> {
        let caps = self
            .0
            .get(role)
            .ok_or(BoardError::Invalid("unknown role"))?;
        if name.trim().is_empty() {
            return Err(BoardError::Invalid("actor name is empty"));
        }
        Ok(Actor {
            name: name.to_owned(),
            caps: caps.clone(),
            projects: None,
        })
    }
}

/// Makes a changed project registry durable (the projects file); see [`Board::set_projects`].
pub type ProjectsSink = Box<dyn Fn(&Projects) -> Result<(), String> + Send + Sync>;

/// The board over a store.
pub struct Board<S> {
    store: S,
    roles: RwLock<Roles>,
    rules: RwLock<MoveRules>,
    workflow: RwLock<Workflow>,
    projects: RwLock<Projects>,
    projects_sink: Option<ProjectsSink>,
    /// Held while numbers are handed out, so this process does not race itself for them.
    /// Uniqueness does not rest on it: the store reserves each number (`Store::reserve_number`).
    numbering: Mutex<()>,
    clock: fn() -> i64,
    /// The decision inbox's source (#595); `None` when no BBP host is configured.
    escalations: Option<Arc<dyn Escalations>>,
    /// The projects' milestones, drawn on the roadmap.
    milestones: Box<dyn MilestoneStore>,
}

/// A read guard even if a writer panicked: the data is plain values, always consistent.
fn read<T: Clone>(lock: &RwLock<T>) -> T {
    lock.read()
        .map_or_else(|e| e.into_inner().clone(), |g| g.clone())
}

/// A card to create (see [`Board::add_card`]).
#[derive(Debug, Clone, Default)]
pub struct NewCard<'a> {
    /// Short title.
    pub title: &'a str,
    /// Description (markdown).
    pub body: &'a str,
    /// Labels.
    pub tags: Vec<String>,
    /// Create it straight in Done, recording work that already merged.
    pub done: bool,
    /// The card it is split from.
    pub parent: Option<Uuid>,
}

impl<'a> NewCard<'a> {
    /// A Backlog card with no parent.
    #[must_use]
    pub fn new(title: &'a str, body: &'a str, tags: Vec<String>) -> Self {
        Self {
            title,
            body,
            tags,
            ..Self::default()
        }
    }
}

/// Wall-clock Unix milliseconds.
pub fn system_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

impl<S: Store> Board<S> {
    /// A board using the system clock.
    pub fn new(store: S, roles: Roles) -> Self {
        Self::with_clock(store, roles, system_ms)
    }

    /// A board with an injected clock (tests).
    pub fn with_clock(store: S, roles: Roles, clock: fn() -> i64) -> Self {
        Self {
            store,
            roles: RwLock::new(roles),
            rules: RwLock::new(MoveRules::default()),
            workflow: RwLock::new(Workflow::default()),
            projects: RwLock::new(Projects::default()),
            projects_sink: None,
            numbering: Mutex::new(()),
            clock,
            escalations: None,
            milestones: Box::new(MemMilestones::default()),
        }
    }

    /// Serve these projects instead of only `default`.
    #[must_use]
    pub fn with_projects(self, projects: Projects) -> Self {
        *self
            .projects
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = projects;
        self
    }

    /// Start with these movement rules instead of [`MoveRules::Enforced`].
    #[must_use]
    pub fn with_move_rules(self, rules: MoveRules) -> Self {
        self.set_move_rules(rules);
        self
    }

    /// Use `workflow` instead of [`Workflow::default`].
    #[must_use]
    pub fn with_workflow(self, workflow: Workflow) -> Self {
        *self
            .workflow
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = workflow;
        self
    }

    /// Keep milestones in `store` instead of in memory.
    #[must_use]
    pub fn with_milestones(mut self, store: Box<dyn MilestoneStore>) -> Self {
        self.milestones = store;
        self
    }

    /// Serve the decision inbox from `source`.
    #[must_use]
    pub fn with_escalations(mut self, source: Arc<dyn Escalations>) -> Self {
        self.escalations = Some(source);
        self
    }

    /// Let [`Board::add_project`] change the registry, persisting it through `sink`.
    #[must_use]
    pub fn with_projects_sink(mut self, sink: ProjectsSink) -> Self {
        self.projects_sink = Some(sink);
        self
    }

    /// Add project `name`, owning `paths`, to repository `repo` (created if new). Additive
    /// only (see [`Projects::with_project`]), so it widens no credential's reach: a credential
    /// limited to some projects does not gain the new one.
    ///
    /// # Errors
    /// If `by` lacks `Project`, the board has no projects file, or the project is refused.
    pub fn add_project(
        &self,
        by: &Actor,
        repo: &str,
        name: &str,
        paths: &[String],
    ) -> Result<Projects, BoardError> {
        if !by.caps.contains(&Capability::Project) {
            return Err(BoardError::Forbidden(Capability::Project));
        }
        let Some(sink) = &self.projects_sink else {
            return Err(BoardError::Refused(
                "this board has no projects file to add a project to".to_owned(),
            ));
        };
        let mut added = None;
        self.update_projects(
            |current| {
                let next = current
                    .with_project(repo, name, paths)
                    .map_err(BoardError::Refused)?;
                added = Some(next.clone());
                Ok(next)
            },
            |p| sink(p),
        )?;
        added.ok_or_else(|| BoardError::Storage("project registry not derived".to_owned()))
    }

    /// A snapshot of the project registry.
    pub fn projects(&self) -> Projects {
        read(&self.projects)
    }

    /// A snapshot of the role registry.
    pub fn roles(&self) -> Roles {
        read(&self.roles)
    }

    /// The registry, held shared: while a guard lives, no project can be removed.
    fn registry(&self) -> RwLockReadGuard<'_, Projects> {
        self.projects
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Replace the project registry. Refused if a card still belongs to a project the new
    /// registry lacks (move or finish those cards first). `persist` makes the new registry
    /// durable and runs under the write lock, after validation and before publication: no
    /// card can be added to or assigned into a project while it is being removed, nor into one
    /// that is not yet durable, and if `persist` fails nothing has changed.
    ///
    /// # Errors
    /// [`BoardError::Refused`] naming the project or what `persist` reported, or a storage failure.
    pub fn set_projects(
        &self,
        projects: Projects,
        persist: impl FnOnce(&Projects) -> Result<(), String>,
    ) -> Result<(), BoardError> {
        self.update_projects(|_| Ok(projects), persist)
    }

    /// [`Board::set_projects`] with the new registry derived from the current one under the
    /// same write lock, so a concurrent change is never lost.
    fn update_projects(
        &self,
        derive: impl FnOnce(&Projects) -> Result<Projects, BoardError>,
        persist: impl FnOnce(&Projects) -> Result<(), String>,
    ) -> Result<(), BoardError> {
        let mut guard = self
            .projects
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let projects = derive(&guard)?;
        if let Some(card) = self
            .store
            .list()?
            .iter()
            .find(|c| guard.contains(&c.project) && !projects.contains(&c.project))
        {
            return Err(BoardError::Refused(format!(
                "project {:?} still has cards (for example {:?}); move them to another project first",
                card.project, card.title
            )));
        }
        persist(&projects).map_err(BoardError::Refused)?;
        *guard = projects;
        Ok(())
    }

    /// Replace the role registry. The caller checks that nobody is locked out.
    pub fn set_roles(&self, roles: Roles) {
        *self
            .roles
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = roles;
    }

    /// The movement rules in force.
    pub fn move_rules(&self) -> MoveRules {
        read(&self.rules)
    }

    /// Turn the movement rules on or off. The caller checks the actor may.
    pub fn set_move_rules(&self, rules: MoveRules) {
        *self
            .rules
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = rules;
    }

    /// The workflow in force: columns and the moves between them.
    pub fn workflow(&self) -> Workflow {
        self.flow().clone()
    }

    /// The workflow, held shared: while a guard lives, [`Board::set_workflow`] waits. Every
    /// write that depends on it (a move, a take) holds it through its commit, so a change to
    /// the workflow never lands between a check and the write it allowed. Never take it twice
    /// in one call: a waiting writer would block the second.
    fn flow(&self) -> RwLockReadGuard<'_, Workflow> {
        self.workflow
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Replace the workflow. Refused if a card is still in a column the new workflow lacks
    /// (move those cards first). `persist` makes it durable and runs under the write lock,
    /// after validation and before publication; if it fails nothing has changed. The write
    /// lock waits for moves and takes in flight and holds new ones until it is published, so
    /// none commits under the workflow it replaces.
    ///
    /// # Errors
    /// [`BoardError::Refused`] naming the column or what `persist` reported, or a storage failure.
    pub fn set_workflow(
        &self,
        next: Workflow,
        persist: impl FnOnce(&Workflow) -> Result<(), String>,
    ) -> Result<(), BoardError> {
        let mut guard = self
            .workflow
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let held = self
            .store
            .list()?
            .into_iter()
            .find(|c| guard.column(&c.column).is_some() && next.column(&c.column).is_none());
        if let Some(card) = held {
            return Err(BoardError::Refused(format!(
                "column {} still has cards (for example {:?}); move them to another column first",
                card.column, card.title
            )));
        }
        persist(&next).map_err(BoardError::Refused)?;
        *guard = next;
        Ok(())
    }

    /// Every card, each [`placed`] in the workflow.
    fn cards(&self) -> Result<Vec<Card>, BoardError> {
        self.cards_in(&self.flow())
    }

    fn cards_in(&self, flow: &Workflow) -> Result<Vec<Card>, BoardError> {
        Ok(self
            .store
            .list()?
            .into_iter()
            .map(|c| placed(flow, c))
            .collect())
    }

    /// The open escalations of the BBP tasks linked from cards `by` may see,
    /// each with its card (design #595 §6). Needs admin: the inbox is the owner's.
    ///
    /// # Errors
    /// [`BoardError::Forbidden`] without admin, [`BoardError::Refused`] when
    /// no BBP host is configured, and storage or host failures.
    pub fn inbox(&self, by: &Actor) -> Result<Vec<InboxItem>, BoardError> {
        let source = self.inbox_source(by)?;
        let cards: BTreeMap<String, Card> = self
            .cards()?
            .into_iter()
            .filter(|c| by.may_work_in(&c.project))
            .filter_map(|c| Some((c.execution.as_ref()?.bbp_task.clone(), c)))
            .collect();
        let tasks: Vec<String> = cards.keys().cloned().collect();
        let open = source.open(&tasks).map_err(inbox_error)?;
        Ok(open
            .into_iter()
            .map(|e| InboxItem {
                card: cards.get(&e.task).map(InboxCard::of),
                label: e.reason.label(),
                escalation: e,
            })
            .collect())
    }

    /// Preview (`dry_run`) or take `action` on escalation `id`, read at `rev`.
    /// The escalation is re-read first: one that has closed, or moved past
    /// `rev`, is refused before anything is sent, and BBP fences the rest.
    ///
    /// # Errors
    /// As [`Board::inbox`], plus [`BoardError::Refused`] for a closed or stale
    /// escalation or an action that does not fit it.
    pub fn inbox_act(
        &self,
        by: &Actor,
        id: &str,
        rev: u64,
        action: &Action,
        dry_run: bool,
    ) -> Result<InboxAnswer, BoardError> {
        let source = self.inbox_source(by)?;
        let Some(InboxItem {
            escalation: e,
            card: owner,
            ..
        }) = self.inbox(by)?.into_iter().find(|i| i.escalation.id == id)
        else {
            return self.closed_escalation(by, id, rev, action, dry_run);
        };
        if e.rev != rev {
            return Err(BoardError::Refused(format!(
                "stale_rev: {id} is at rev {}, not {rev}; re-read the inbox",
                e.rev
            )));
        }
        if dry_run {
            return source
                .preview(&e, action)
                .map(InboxAnswer::DryRun)
                .map_err(inbox_error);
        }
        let dropped = e.dropped_acted(action).map_err(inbox_error)?;
        if dropped.is_empty() {
            let outcome = source
                .act(&e, action, &answerer(&by.name))
                .map_err(inbox_error)?;
            return Ok(InboxAnswer::Outcome(outcome));
        }
        if !self.store.durable() {
            return Err(BoardError::Refused(
                "this decision drops acted-on work, and its Remediate card must be written to a durable store first: start the board with --tick-url".into(),
            ));
        }
        // Write ahead: the Remediate card is committed before BBP is asked to drop the
        // work, so no drop sent from here can lose its card. A retry finds the card by
        // its derived id, whether BBP's answer was lost or the escalation is already gone.
        let dry = source.preview(&e, action).map_err(inbox_error)?;
        let project = owner.map_or_else(|| DEFAULT_PROJECT.to_owned(), |c| c.project);
        let decision = Decision {
            escalation: id,
            rev,
            action,
        };
        let card = self.remediate(by, &project, &e, &decision, &dropped, &dry)?;
        let outcome =
            source
                .act(&e, action, &answerer(&by.name))
                .map_err(|err| match inbox_error(err) {
                    BoardError::Storage(why) => BoardError::Storage(format!(
                        "{why}; its Remediate card {} was written first, so retry to reconcile",
                        card.label()
                    )),
                    other => other,
                })?;
        if let Outcome::Refused { step: 0, code, .. } = &outcome {
            // Only this send is known to have written nothing: another send of the same
            // decision shares the card and may commit before or after any re-read.
            self.comment(
                card.id,
                by,
                &format!("BBP refused this send ({code}). Another send of the same decision may still apply it, so check BBP's log before closing this card."),
            )?;
        }
        Ok(InboxAnswer::Remediated { outcome, card })
    }

    /// An escalation that is no longer open: if this exact decision was sent from here
    /// and dropped acted-on work, its Remediate card is the answer to a retry.
    fn closed_escalation(
        &self,
        by: &Actor,
        id: &str,
        rev: u64,
        action: &Action,
        dry_run: bool,
    ) -> Result<InboxAnswer, BoardError> {
        let closed = || BoardError::Refused(format!("no open escalation {id}"));
        if dry_run {
            return Err(closed());
        }
        let task = id.split('/').next().unwrap_or_default();
        let project = self.owning_project(by, task)?;
        let decision = Decision {
            escalation: id,
            rev,
            action,
        };
        self.remediation_card(by, &project, &decision)?
            .map(|card| InboxCard::of(&card))
            .map(|card| InboxAnswer::Recorded { card })
            .ok_or_else(closed)
    }

    /// The project of the card linked to BBP task `task`, as `inbox` attaches it.
    fn owning_project(&self, by: &Actor, task: &str) -> Result<String, BoardError> {
        Ok(self
            .cards()?
            .into_iter()
            .filter(|c| by.may_work_in(&c.project))
            .find(|c| c.execution.as_ref().is_some_and(|x| x.bbp_task == task))
            .map_or_else(|| DEFAULT_PROJECT.to_owned(), |c| c.project))
    }

    /// The Remediate card written for `decision` in `project`. It is found by its
    /// derived id, never by its tag: a tag anyone can add to any card is no record.
    fn remediation_card(
        &self,
        by: &Actor,
        project: &str,
        decision: &Decision<'_>,
    ) -> Result<Option<Card>, BoardError> {
        if !by.may_work_in(project) {
            return Ok(None);
        }
        Ok(self
            .store
            .get(decision.card_id(project)?)?
            .filter(|c| c.project == project))
    }

    /// The one Backlog "Remediate" card for `decision`'s dropped acted-on work (#595 §4),
    /// at the id derived from `project` and the exact decision, so a retry of it finds
    /// the card rather than adding a second, and a different decision never reuses it.
    /// Its `remediate:` tag is for people only.
    fn remediate(
        &self,
        by: &Actor,
        project: &str,
        e: &Escalation,
        decision: &Decision<'_>,
        dropped: &[&Acted],
        dry: &DryRun,
    ) -> Result<InboxCard, BoardError> {
        if let Some(card) = self.remediation_card(by, project, decision)? {
            return Ok(InboxCard::of(&card));
        }
        let tag = format!("remediate:{}", e.id);
        let text = |id: u64| {
            e.candidates
                .iter()
                .chain(e.claim.as_ref())
                .find(|c| c.id == id)
                .map_or_else(|| format!("msg:{id}"), |c| c.text.clone())
        };
        let mut body = format!(
            "The decision on BBP escalation `{}` dropped work that was already acted on. Undo or reconcile it.\n\n",
            e.id
        );
        for x in dropped {
            body.push_str(&format!(
                "- msg:{} \"{}\": {}\n",
                x.claim,
                text(x.claim),
                x.marks.join(", ")
            ));
        }
        let events: Vec<&str> = dry
            .events
            .iter()
            .filter_map(|v| v.as_object()?.keys().next().map(String::as_str))
            .collect();
        body.push_str(&format!(
            "\nBBP dry run of `{} {}`: {}\n",
            dry.step.verb,
            dry.step.args.join(" "),
            events.join(", ")
        ));
        body.push_str("\nFiled before the decision was sent to BBP. If BBP's log shows it was not applied, close this card.\n");
        let title = format!(
            "Remediate: {}",
            text(dropped.first().map_or(0, |x| x.claim))
        );
        let card = Card {
            id: decision.card_id(project)?,
            ..Card::new(&title, &body, vec![tag], (self.clock)())?
        };
        match self.create(by, project, card) {
            Ok(card) => Ok(InboxCard::of(&card)),
            // A concurrent retry of the same decision wrote it first.
            Err(BoardError::Conflict) => self
                .remediation_card(by, project, decision)?
                .map(|c| InboxCard::of(&c))
                .ok_or(BoardError::Conflict),
            Err(other) => Err(other),
        }
    }

    fn inbox_source(&self, by: &Actor) -> Result<&Arc<dyn Escalations>, BoardError> {
        if !by.caps.contains(&Capability::Admin) {
            return Err(BoardError::Forbidden(Capability::Admin));
        }
        self.escalations.as_ref().ok_or_else(|| {
            BoardError::Refused(
                "the inbox has no BBP host: start with --bbp-bin and --bbp-root".into(),
            )
        })
    }

    /// All cards, optionally one column only.
    ///
    /// # Errors
    /// On storage failure.
    pub fn list(&self, column: Option<&Column>) -> Result<Vec<Card>, BoardError> {
        let mut cards = self.cards()?;
        cards.retain(|c| column.is_none_or(|col| c.column == *col));
        Ok(cards)
    }

    /// As [`Board::list`], keeping only the cards `shelf` shows when Done cards leave the
    /// default views after `window_days` (`0`: only when archived).
    ///
    /// # Errors
    /// On storage failure.
    pub fn list_shelf(
        &self,
        column: Option<&Column>,
        shelf: Shelf,
        window_days: u32,
    ) -> Result<Vec<Card>, BoardError> {
        let now = (self.clock)();
        let window = window_ms(window_days);
        let mut cards = self.list(column)?;
        cards.retain(|c| shelf.keeps(c.shelved(now, window)));
        Ok(cards)
    }

    /// Keep the `cards` that `dates` selects. Moves come from the store's history.
    ///
    /// # Errors
    /// If the store fails.
    pub fn retain_dates(&self, cards: &mut Vec<Card>, dates: &Dates) -> Result<(), BoardError> {
        cards.retain(|c| {
            dates.created.contains(c.created_ms) && dates.updated.contains(c.updated_ms)
        });
        if dates.moved.is_bounded() {
            let moved = self.store.moved_in(dates.moved)?;
            cards.retain(|c| moved.contains(&c.id));
        }
        Ok(())
    }

    /// The roadmap of `project` as of now (see [`roadmap`](crate::roadmap::roadmap)).
    ///
    /// # Errors
    /// If the store fails.
    pub fn roadmap(&self, project: &str) -> Result<Roadmap, BoardError> {
        Ok(roadmap(
            &self.workflow(),
            &self.cards()?,
            project,
            (self.clock)(),
        ))
    }

    /// The milestones `keep` selects, by target date.
    ///
    /// # Errors
    /// If the store fails.
    pub fn milestones(
        &self,
        keep: impl Fn(&Milestone) -> bool,
    ) -> Result<Vec<Milestone>, BoardError> {
        let mut all = self.milestones.milestones()?;
        all.retain(|m| keep(m));
        all.sort_by(|a, z| (a.target_ms, &a.title).cmp(&(z.target_ms, &z.title)));
        Ok(all)
    }

    /// Each of `milestones` with the progress of the cards linked to it.
    ///
    /// # Errors
    /// If the store fails.
    pub fn milestone_progress(
        &self,
        milestones: Vec<Milestone>,
    ) -> Result<Vec<MilestoneView>, BoardError> {
        Ok(trace::with_progress(&self.cards()?, milestones))
    }

    /// Add a milestone to `project`. Needs `Triage`: dates are planning decisions.
    ///
    /// # Errors
    /// A missing capability, an unknown project, or see [`Milestone::new`].
    pub fn add_milestone(
        &self,
        by: &Actor,
        project: &str,
        title: &str,
        target_ms: i64,
    ) -> Result<Milestone, BoardError> {
        if !by.caps.contains(&Capability::Triage) {
            return Err(BoardError::Forbidden(Capability::Triage));
        }
        if !self.projects().contains(project) {
            return Err(BoardError::Invalid("unknown project"));
        }
        let mut milestone = Milestone::new(project, title, target_ms)?;
        milestone.rev = self.milestones.add_milestone(&milestone)?;
        Ok(milestone)
    }

    /// Change a milestone: its title, target, or whether it is done. Needs `Triage`; `in_scope`
    /// says whether the caller may touch its project.
    ///
    /// The save is fenced on the version the scope check read, so a milestone moved to another
    /// project meanwhile is refused with [`BoardError::Conflict`] rather than written.
    ///
    /// # Errors
    /// A missing capability, a missing or out-of-scope milestone, a conflict, or see
    /// [`Milestone::edited`].
    pub fn edit_milestone(
        &self,
        by: &Actor,
        id: Uuid,
        edit: &MilestoneEdit,
        in_scope: impl Fn(&str) -> bool,
    ) -> Result<Milestone, BoardError> {
        if !by.caps.contains(&Capability::Triage) {
            return Err(BoardError::Forbidden(Capability::Triage));
        }
        let current = self
            .milestones
            .milestone(id)?
            .ok_or(BoardError::NotFound(id))?;
        if !in_scope(&current.project) {
            return Err(BoardError::OutOfScope(current.project));
        }
        let mut next = current.edited(edit, (self.clock)())?;
        next.rev = self.milestones.save_milestone(&current, &next)?;
        Ok(next)
    }

    /// The metrics, as of now, of the cards `keep` selects (see
    /// [`metrics`](crate::metrics::metrics)).
    ///
    /// # Errors
    /// If the store fails.
    pub fn metrics(&self, keep: impl Fn(&Card) -> bool) -> Result<Metrics, BoardError> {
        let mut cards = self.cards()?;
        cards.retain(|c| keep(c));
        Ok(metrics(&self.workflow(), &cards, (self.clock)()))
    }

    /// One card.
    ///
    /// # Errors
    /// If missing, or on storage failure.
    pub fn get(&self, id: Uuid) -> Result<Card, BoardError> {
        self.get_in(&self.flow(), id)
    }

    fn get_in(&self, flow: &Workflow, id: Uuid) -> Result<Card, BoardError> {
        self.store
            .get(id)?
            .map(|c| placed(flow, c))
            .ok_or(BoardError::NotFound(id))
    }

    /// The card numbered `number`.
    ///
    /// # Errors
    /// If no card has that number, if more than one has (a number tag edited by hand in
    /// rusty_tick): never a guess at which was meant. Or on storage failure.
    pub fn get_number(&self, number: u64) -> Result<Card, BoardError> {
        let mut found = self.store.numbered(number)?.into_iter();
        match (found.next(), found.next()) {
            (Some(card), None) => Ok(card),
            (None, _) => Err(BoardError::Invalid("no card has that number")),
            (Some(_), Some(_)) => Err(BoardError::Invalid(
                "more than one card has that number; name it by id",
            )),
        }
    }

    /// Number every card that has none yet (made before numbering, or directly in rusty_tick),
    /// oldest first. Returns how many were numbered.
    ///
    /// # Errors
    /// On storage failure; cards numbered before it keep their numbers.
    pub fn number_cards(&self) -> Result<usize, BoardError> {
        let _numbering = self
            .numbering
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        self.number_strays().map(|(numbered, _)| numbered)
    }

    /// Number the cards that have none, each with a number the store reserves; returns how many
    /// it numbered and the highest number on the board. The caller holds `numbering`.
    fn number_strays(&self) -> Result<(usize, u64), BoardError> {
        let numberer = Actor {
            name: NUMBERER.into(),
            caps: BTreeSet::new(),
            projects: None,
        };
        let cards = self.store.list()?;
        let mut highest = cards.iter().filter_map(|c| c.number).max().unwrap_or(0);
        let strays: Vec<Uuid> = cards
            .iter()
            .filter(|c| c.number.is_none())
            .map(|c| c.id)
            .collect();
        for id in &strays {
            highest = self.store.reserve_number(*id, highest)?;
            // Another board may number the card first; then this reservation goes unused.
            self.apply(*id, &numberer, EventKind::Numbered, |c, _| {
                Ok(c.numbered(highest))
            })?;
        }
        Ok((strays.len(), highest))
    }

    /// Commit a new card under a newly reserved number.
    fn insert_numbered(&self, by: &Actor, mut card: Card) -> Result<Card, BoardError> {
        let _numbering = self
            .numbering
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let highest = self.number_strays()?.1;
        card.number = Some(self.store.reserve_number(card.id, highest)?);
        card.rev = self
            .store
            .insert(&card, Event::of(&card, EventKind::Created, &by.name))?;
        Ok(card)
    }

    /// Events after the cursor.
    ///
    /// # Errors
    /// On storage failure.
    pub fn events(&self, after: Option<Uuid>, limit: usize) -> Result<Vec<Event>, BoardError> {
        self.store.events_since(after, limit)
    }

    /// The newest event id: a cursor that has seen everything committed so far.
    ///
    /// # Errors
    /// On storage failure.
    pub fn cursor(&self) -> Result<Option<Uuid>, BoardError> {
        self.store.last_event_id()
    }

    /// `Err` once the store can no longer commit and must be reopened.
    ///
    /// # Errors
    /// On storage failure.
    pub fn health(&self) -> Result<(), BoardError> {
        self.store.health()
    }

    /// Create a Backlog card in the `default` project.
    ///
    /// # Errors
    /// On a blank title or storage failure.
    pub fn add(
        &self,
        by: &Actor,
        title: &str,
        body: &str,
        tags: Vec<String>,
    ) -> Result<Card, BoardError> {
        self.add_in(by, DEFAULT_PROJECT, title, body, tags)
    }

    /// Create a Backlog card in `project`.
    ///
    /// # Errors
    /// On a blank title, an unknown project, a project outside `by`'s scope, or
    /// storage failure.
    pub fn add_in(
        &self,
        by: &Actor,
        project: &str,
        title: &str,
        body: &str,
        tags: Vec<String>,
    ) -> Result<Card, BoardError> {
        self.add_card(by, project, NewCard::new(title, body, tags))
    }

    /// Create a card straight in Done in `project`, recording work that
    /// already merged outside the board's flow. Needs `Close`.
    ///
    /// # Errors
    /// As [`Board::add_in`], or if `by` lacks `Close`.
    pub fn add_done_in(
        &self,
        by: &Actor,
        project: &str,
        title: &str,
        body: &str,
        tags: Vec<String>,
    ) -> Result<Card, BoardError> {
        let new = NewCard {
            done: true,
            ..NewCard::new(title, body, tags)
        };
        self.add_card(by, project, new)
    }

    /// Create a card in `project` as `new` describes: in Backlog, or in Done (needs `Close`),
    /// optionally split from a `parent` card. Naming the parent needs no `Triage`: whoever splits
    /// work records where each piece came from.
    ///
    /// # Errors
    /// As [`Board::add_in`], a missing `Close` for a Done card, or a parent that is not a card.
    pub fn add_card(
        &self,
        by: &Actor,
        project: &str,
        new: NewCard<'_>,
    ) -> Result<Card, BoardError> {
        if new.done && !by.caps.contains(&Capability::Close) {
            return Err(BoardError::Forbidden(Capability::Close));
        }
        let mut card = Card::new(new.title, new.body, new.tags, (self.clock)())?;
        if new.done {
            card.column = Column::DONE;
            card.done_ms = Some(card.created_ms);
        }
        if let Some(p) = new.parent {
            // A new card has no children, so no parent can close a loop through it.
            self.store.get(p)?.ok_or(BoardError::NotFound(p))?;
            card.parent = Some(p);
        }
        self.create(by, project, card)
    }

    fn create(&self, by: &Actor, project: &str, mut card: Card) -> Result<Card, BoardError> {
        let registry = self.registry();
        if !registry.contains(project) {
            return Err(BoardError::Invalid("unknown project"));
        }
        if !by.may_work_in(project) {
            return Err(BoardError::OutOfScope(project.to_owned()));
        }
        project.clone_into(&mut card.project);
        let card = self.insert_numbered(by, card)?;
        drop(registry);
        Ok(card)
    }

    /// Create a Backlog card for an existing forge issue.
    ///
    /// # Errors
    /// On a blank title or storage failure.
    pub fn import(
        &self,
        by: &Actor,
        title: &str,
        body: &str,
        tags: Vec<String>,
        issue: u64,
    ) -> Result<Card, BoardError> {
        let mut card = Card::new(title, body, tags, (self.clock)())?;
        card.id = import_id(issue);
        card.issue = Some(issue);
        card.synced_text = Some(text_hash(&card.title, &card.body));
        self.insert_numbered(by, card)
    }

    /// Record the forge issue a card is synced with, and the text hash both
    /// sides agreed on at that moment (`None` when unknown).
    ///
    /// # Errors
    /// If the card is missing or on storage failure.
    pub fn link_issue(
        &self,
        id: Uuid,
        issue: u64,
        synced_text: Option<u64>,
        by: &Actor,
    ) -> Result<Card, BoardError> {
        self.apply(id, by, EventKind::Linked, |c, now| {
            Ok(c.link(issue, synced_text, now))
        })
    }

    /// Record that the card's title and body match its issue (hash `synced_text`).
    ///
    /// # Errors
    /// If the card is missing or on storage failure.
    pub fn mark_synced(&self, id: Uuid, synced_text: u64, by: &Actor) -> Result<Card, BoardError> {
        self.apply(id, by, EventKind::Synced, |c, now| {
            Ok(c.mark_synced(synced_text, now))
        })
    }

    /// Persist or adopt the one factory execution linked to this card. Builders
    /// may start work, but cannot replace an existing logical execution.
    pub fn bind_execution(
        &self,
        id: Uuid,
        link: ExecutionLink,
        by: &Actor,
    ) -> Result<Card, BoardError> {
        if !by.caps.contains(&Capability::Build) {
            return Err(BoardError::Forbidden(Capability::Build));
        }
        self.apply(id, by, EventKind::ExecutionLinked, |c, now| {
            c.bind_execution(link.clone(), now)
        })
    }

    /// Move this card's execution to a replacement assignment generation, if
    /// the stored link still carries `expected_generation` (see
    /// [`Card::recover_execution`]). Needs Build, like binding.
    pub fn recover_execution(
        &self,
        id: Uuid,
        expected_generation: u64,
        link: ExecutionLink,
        by: &Actor,
    ) -> Result<Card, BoardError> {
        if !by.caps.contains(&Capability::Build) {
            return Err(BoardError::Forbidden(Capability::Build));
        }
        self.apply(id, by, EventKind::ExecutionRecovered, |c, now| {
            c.recover_execution(expected_generation, link.clone(), now)
        })
    }

    /// Pull work: the next card this actor can act on, entered (Ready ->
    /// Building) and claimed. With `id`, that card or an error. Cards nearest
    /// Done come first, then oldest. Cards claimed by someone else are
    /// skipped; the actor's own claims are resumed. `Ok(None)` when nothing
    /// is waiting for them.
    ///
    /// # Errors
    /// On storage failure, or (with `id`) whatever refused that card.
    pub fn take(&self, by: &Actor, id: Option<Uuid>) -> Result<Option<Card>, BoardError> {
        self.take_in(by, id, None)
    }

    /// As [`Board::take`], restricted to cards of `project` when given. A card of a project
    /// outside `by`'s scope is refused by the write itself (see `apply`), so it is skipped
    /// when scanning and an error when named by `id`.
    ///
    /// # Errors
    /// As [`Board::take`], plus an unknown or out-of-scope `project`.
    pub fn take_in(
        &self,
        by: &Actor,
        id: Option<Uuid>,
        project: Option<&str>,
    ) -> Result<Option<Card>, BoardError> {
        if let Some(p) = project {
            if !self.registry().contains(p) {
                return Err(BoardError::Invalid("unknown project"));
            }
            if !by.may_work_in(p) {
                return Err(BoardError::OutOfScope(p.to_owned()));
            }
        }
        let flow = self.flow();
        let queues = flow.work_queues();
        let rank = |c: &Card| queues.iter().position(|q| *q == c.column);
        let cards = self.cards_in(&flow)?;
        let waiting = waiting_on_deps(&cards);
        if let Some(i) = id
            && waiting.contains(&i)
        {
            return Err(waits_on_deps());
        }
        if let Some(named) = id.and_then(|i| cards.iter().find(|c| c.id == i)) {
            check_not_superseded(named)?;
        }
        let mut candidates: Vec<Card> = cards
            .into_iter()
            .filter(|c| id.is_none_or(|i| c.id == i))
            .filter(|c| project.is_none_or(|p| c.project == p))
            .filter(|c| rank(c).is_some() && flow.actionable(&c.column, &by.caps))
            .filter(|c| id.is_some() || c.claim.as_deref().is_none_or(|h| h == by.name))
            .filter(|c| !waiting.contains(&c.id))
            .filter(|c| c.superseded_by.is_empty())
            .collect();
        // Nearest Done first; within a queue the highest priority, then the manual order,
        // then the oldest.
        candidates.sort_by_key(|c| (rank(c), Reverse(c.priority), c.sort_order, c.created_ms));
        for card in candidates {
            match self.enter_and_claim(&flow, card, by) {
                Ok(card) => return Ok(Some(card)),
                // Another worker got there first and the card has moved on or been claimed:
                // try the next one. (With an explicit `id`, say why instead.)
                Err(
                    BoardError::Conflict
                    | BoardError::Claimed(_)
                    | BoardError::NoSuchMove { .. }
                    | BoardError::Forbidden(_)
                    | BoardError::NotClaimant
                    | BoardError::Terminal
                    | BoardError::OutOfScope(_)
                    | BoardError::NotFound(_)
                    | BoardError::Refused(_),
                ) if id.is_none() => {}
                Err(e) => return Err(e),
            }
        }
        match id {
            Some(i) => Err(BoardError::NotFound(i)),
            None => Ok(None),
        }
    }

    /// Enter (if the card waits in a queue) and claim it, in one commit, so a take that is
    /// refused leaves the card where it was. The commit re-checks the card *inside* its
    /// compare-and-swap, so a card another worker moved on or claimed since it was listed is
    /// refused instead of half-taken; and a card is entered only into a column whose work
    /// this actor does, so it is never moved and then left unclaimed.
    fn enter_and_claim(
        &self,
        flow: &Workflow,
        listed: Card,
        by: &Actor,
    ) -> Result<Card, BoardError> {
        let id = listed.id;
        let queues = flow.work_queues();
        let works = |col: &Column| queues.contains(col) && flow.actionable(col, &by.caps);
        let current = self.get_in(flow, id)?;
        if !works(&current.column) {
            return Err(BoardError::Conflict);
        }
        self.check_takeable(&current)?;
        // Refuse before writing anything: a claimed card must not be half-taken.
        if let Some(holder) = current.claim.as_ref().filter(|h| **h != by.name) {
            return Err(BoardError::Claimed(holder.clone()));
        }
        let Some(enter) = flow.entry_column(&current.column) else {
            return self.apply_in(flow, id, by, EventKind::Claimed, |c, now| {
                if !works(&c.column) {
                    return Err(BoardError::Conflict);
                }
                self.check_takeable(c)?;
                c.claim(by, now)
            });
        };
        if !works(enter) {
            return Err(BoardError::Refused(format!(
                "taking this card moves it to {enter}, whose work your role does not do"
            )));
        }
        self.apply_in(flow, id, by, EventKind::Moved, |c, now| {
            // Still in the queue it was taken from: a card moved meanwhile to another queue
            // (even one entering the same column) may not be this actor's to take.
            if c.column != current.column {
                return Err(BoardError::Conflict);
            }
            self.check_takeable(c)?;
            // Checked on the card being committed, not the earlier read.
            if let Some(holder) = c.claim.as_ref().filter(|h| **h != by.name) {
                return Err(BoardError::Claimed(holder.clone()));
            }
            let entered = c.move_to(flow, enter, by, now)?;
            Ok(Card {
                claim: Some(by.name.clone()),
                ..entered
            })
        })
    }

    /// Refuse `card` while it is superseded or waits on a dependency. Checked on the card
    /// being committed, so links planned after `take` listed it still count.
    fn check_takeable(&self, card: &Card) -> Result<(), BoardError> {
        check_not_superseded(card)?;
        self.check_deps_done(card)
    }

    /// Refuse `card` while any of its dependencies is not Done. Checked on the card being
    /// committed, so dependencies planned after `take` listed it still count; a Done
    /// dependency stays Done (it is terminal), so the check cannot go stale.
    fn check_deps_done(&self, card: &Card) -> Result<(), BoardError> {
        for dep in &card.deps {
            if self
                .store
                .get(*dep)?
                .is_some_and(|d| d.column != Column::DONE)
            {
                return Err(waits_on_deps());
            }
        }
        Ok(())
    }

    /// Apply `op` to the card and commit, re-reading on conflict.
    fn apply(
        &self,
        id: Uuid,
        by: &Actor,
        kind: EventKind,
        op: impl Fn(&Card, i64) -> Result<Card, BoardError>,
    ) -> Result<Card, BoardError> {
        self.apply_in(&self.flow(), id, by, kind, op)
    }

    /// [`Board::apply`] under `flow`, a guard the caller holds (see [`Board::flow`]).
    fn apply_in(
        &self,
        flow: &Workflow,
        id: Uuid,
        by: &Actor,
        kind: EventKind,
        op: impl Fn(&Card, i64) -> Result<Card, BoardError>,
    ) -> Result<Card, BoardError> {
        for _ in 0..RETRIES {
            let current = self.get_in(flow, id)?;
            // Checked on the card being committed, so a card re-assigned mid-call is refused too.
            if !by.may_work_in(&current.project) {
                return Err(BoardError::OutOfScope(current.project));
            }
            let next = op(&current, (self.clock)())?;
            // Domain operations may deliberately return the unchanged aggregate
            // for an idempotent replay.  Such a replay is already durable and
            // must not be sent through the store's rev + 1 CAS.
            if next == current {
                return Ok(current);
            }
            match self.store.replace(&next, Event::of(&next, kind, &by.name)) {
                Err(BoardError::Conflict) => continue,
                // The store's revision, not ours: an adapter that commits in two
                // steps moves it twice, and the caller's next edit names it.
                result => return result.map(|rev| Card { rev, ..next }),
            }
        }
        Err(BoardError::Conflict)
    }

    /// Move a card, under the movement rules in force. A move made with the rules off is
    /// logged, so what bypassed the table can be traced.
    ///
    /// # Errors
    /// See [`Card::move_to`], or [`Card::move_freely`] when the rules are off.
    pub fn move_card(&self, id: Uuid, to: &Column, by: &Actor) -> Result<Card, BoardError> {
        let flow = self.flow();
        match self.move_rules() {
            MoveRules::Enforced => self.apply_in(&flow, id, by, EventKind::Moved, |c, now| {
                c.move_to(&flow, to, by, now)
            }),
            MoveRules::Open => {
                let card = self.apply_in(&flow, id, by, EventKind::Moved, |c, now| {
                    c.move_freely(&flow, to, by, now)
                })?;
                rusty_log::info!(card = %id, actor = %by.name, to = %to, "card moved with movement rules off");
                Ok(card)
            }
        }
    }

    /// Move a card to another project. Needs `Triage`, or else `Project` (`Close` for a Done
    /// card, which stays Done), and both projects in scope; without `Triage`, not a card
    /// someone else has claimed.
    ///
    /// # Errors
    /// An unknown project, a missing capability, an out-of-scope project, or a card claimed
    /// by someone else (without `Triage`).
    pub fn assign(&self, id: Uuid, project: &str, by: &Actor) -> Result<Card, BoardError> {
        let registry = self.registry();
        if !registry.contains(project) {
            return Err(BoardError::Invalid("unknown project"));
        }
        if !by.may_work_in(project) {
            return Err(BoardError::OutOfScope(project.to_owned()));
        }
        let triage = by.caps.contains(&Capability::Triage);
        let card = self.apply(id, by, EventKind::Assigned, |c, now| {
            let needed = assign_capability(&c.column);
            if !triage && !by.caps.contains(&needed) {
                return Err(BoardError::Forbidden(needed));
            }
            match &c.claim {
                Some(holder) if !triage && *holder != by.name => {
                    Err(BoardError::Claimed(holder.clone()))
                }
                _ => Ok(c.assign(project, now)),
            }
        });
        drop(registry);
        card
    }

    /// Archive a Done card. Needs `Close`, as closing work does.
    ///
    /// # Errors
    /// A missing capability, or see [`Card::archive`].
    pub fn archive(&self, id: Uuid, by: &Actor) -> Result<Card, BoardError> {
        if !by.caps.contains(&Capability::Close) {
            return Err(BoardError::Forbidden(Capability::Close));
        }
        self.apply(id, by, EventKind::Archived, |c, now| c.archive(now))
    }

    /// Put a shelved card (archived, or past the default Done window) back on the default
    /// views: see [`Card::restore`]. Needs `Close`.
    ///
    /// # Errors
    /// A missing capability, or a missing card.
    pub fn restore(&self, id: Uuid, by: &Actor) -> Result<Card, BoardError> {
        if !by.caps.contains(&Capability::Close) {
            return Err(BoardError::Forbidden(Capability::Close));
        }
        let window = window_ms(DONE_WINDOW_DAYS);
        self.apply(id, by, EventKind::Restored, |c, now| {
            Ok(c.restore(now, window))
        })
    }

    /// Archive every Done card not yet archived whose window ([`Card::shelf_since_ms`])
    /// started at least `older_than_days` ago, in `project` or in every project `by` may work
    /// in. Needs `Triage`: it is a board-wide tidy-up. Returns the cards archived.
    ///
    /// The selection is checked again on the card each commit is made from, so a card moved to
    /// another project, archived, restored or removed meanwhile is skipped, never archived.
    ///
    /// # Errors
    /// A missing capability, or a storage failure; the cards archived before it stay archived.
    pub fn archive_done(
        &self,
        by: &Actor,
        project: Option<&str>,
        older_than_days: u32,
    ) -> Result<Vec<Card>, BoardError> {
        if !by.caps.contains(&Capability::Triage) {
            return Err(BoardError::Forbidden(Capability::Triage));
        }
        let window = window_ms(older_than_days).unwrap_or(0);
        let wanted = |c: &Card, now: i64| {
            c.column == Column::DONE
                && c.archived_ms.is_none()
                && by.may_work_in(&c.project)
                && project.is_none_or(|p| c.project == p)
                && now.saturating_sub(c.shelf_since_ms()) >= window
        };
        let listed_at = (self.clock)();
        let mut archived = Vec::new();
        for card in self.list(Some(&Column::DONE))? {
            if !wanted(&card, listed_at) {
                continue;
            }
            let took = Cell::new(false);
            let result = self.apply(card.id, by, EventKind::Archived, |c, now| {
                // Each attempt decides afresh on the card it read.
                took.set(wanted(c, now));
                if took.get() {
                    c.archive(now)
                } else {
                    Ok(c.clone())
                }
            });
            match result {
                Ok(c) if took.get() => archived.push(c),
                Ok(_) | Err(BoardError::NotFound(_) | BoardError::OutOfScope(_)) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(archived)
    }

    /// Claim a card.
    ///
    /// # Errors
    /// See [`Card::claim`].
    pub fn claim(&self, id: Uuid, by: &Actor) -> Result<Card, BoardError> {
        self.apply(id, by, EventKind::Claimed, |c, now| c.claim(by, now))
    }

    /// Release a claim.
    ///
    /// # Errors
    /// See [`Card::release`].
    pub fn release(&self, id: Uuid, by: &Actor) -> Result<Card, BoardError> {
        self.apply(id, by, EventKind::Released, |c, now| c.release(by, now))
    }

    /// Edit title/body/tags.
    ///
    /// # Errors
    /// See [`Card::edit`].
    pub fn edit(
        &self,
        id: Uuid,
        by: &Actor,
        title: Option<&str>,
        body: Option<&str>,
        tags: Option<Vec<String>>,
    ) -> Result<Card, BoardError> {
        let edit = Edit {
            title,
            body,
            tags,
            prompt: None,
        };
        self.edit_at(id, by, None, &edit)
    }

    /// As [`Board::edit`], but only if the card is still at revision `expect_rev`
    /// (what the editor last saw); otherwise [`BoardError::Conflict`], so an
    /// editor never silently overwrites changes it has not seen.
    ///
    /// # Errors
    /// See [`Card::edit`], and [`BoardError::Conflict`] on a stale revision.
    pub fn edit_at(
        &self,
        id: Uuid,
        by: &Actor,
        expect_rev: Option<u64>,
        edit: &Edit<'_>,
    ) -> Result<Card, BoardError> {
        self.apply(id, by, EventKind::Edited, |c, now| {
            if expect_rev.is_some_and(|rev| c.rev != rev) {
                return Err(BoardError::Conflict);
            }
            let next = c.edit(edit.title, edit.body, edit.tags.clone(), now)?;
            match edit.prompt {
                Some(p) => next.with_prompt(p),
                None => Ok(next),
            }
        })
    }

    /// Set a card's priority and links (see [`Plan`]). Needs `Triage`: what is taken
    /// first, what waits, and what a newer card replaces are planning decisions.
    ///
    /// # Errors
    /// A missing capability, an empty plan, a linked id that is not a card, or see
    /// [`Card::plan`].
    pub fn plan(&self, id: Uuid, by: &Actor, plan: &Plan) -> Result<Card, BoardError> {
        if !by.caps.contains(&Capability::Triage) {
            return Err(BoardError::Forbidden(Capability::Triage));
        }
        if plan.is_empty() {
            return Err(BoardError::Invalid(
                "give a priority, deps, superseded_by, related, parent, milestone or target_ms",
            ));
        }
        for linked in plan.linked() {
            if *linked != id && self.store.get(*linked)?.is_none() {
                return Err(BoardError::NotFound(*linked));
            }
        }
        if let Some(Some(parent)) = plan.parent {
            self.check_parent(id, parent)?;
        }
        if let Some(Some(m)) = plan.milestone {
            let card = self.get(id)?;
            self.check_milestone(m, &card.project)?;
        }
        // One commit: `Card::plan` validates the whole request before anything changes.
        self.apply(id, by, EventKind::Edited, |c, now| c.plan(plan, now))
    }

    /// Refuse `parent` for card `id` when it would close a loop or the chain is too deep.
    fn check_parent(&self, id: Uuid, parent: Uuid) -> Result<(), BoardError> {
        trace::check_parent(id, parent, |c| {
            Ok(self.store.get(c)?.and_then(|c| c.parent))
        })
    }

    /// Refuse a milestone that does not exist or is in another project than the card's.
    fn check_milestone(&self, id: Uuid, project: &str) -> Result<(), BoardError> {
        let m = self
            .milestones
            .milestone(id)?
            .ok_or(BoardError::NotFound(id))?;
        if m.project != project {
            return Err(BoardError::Invalid(
                "a card links only to a milestone of its own project",
            ));
        }
        Ok(())
    }

    /// Where card `id` sits in its decomposition: its chain of parents and the tree below it,
    /// leaving out cards `keep` refuses (see [`trace::trace`]).
    ///
    /// # Errors
    /// A missing card, or if the store fails.
    pub fn trace(&self, id: Uuid, keep: impl Fn(&Card) -> bool) -> Result<Trace, BoardError> {
        let cards = self.cards()?;
        let card = cards
            .iter()
            .find(|c| c.id == id)
            .ok_or(BoardError::NotFound(id))?;
        Ok(trace::trace(&cards, card, keep))
    }

    /// Comment on a card.
    ///
    /// # Errors
    /// See [`Card::comment`].
    pub fn comment(&self, id: Uuid, by: &Actor, text: &str) -> Result<Card, BoardError> {
        self.apply(id, by, EventKind::Commented, |c, now| {
            c.comment(by, text, now)
        })
    }
}

/// `days` as milliseconds, or `None` for `0`: no window.
fn window_ms(days: u32) -> Option<i64> {
    (days > 0).then(|| i64::from(days) * 24 * 60 * 60 * 1000)
}

/// `card` as `flow` sees it: in Backlog if its column is not one of the workflow's (removed
/// while the card was moving in, or a tag edited by hand in rusty_tick).
fn placed(flow: &Workflow, card: Card) -> Card {
    if flow.column(&card.column).is_some() {
        return card;
    }
    Card {
        column: Column::BACKLOG,
        ..card
    }
}

fn waits_on_deps() -> BoardError {
    BoardError::Refused("card waits on cards that are not done".into())
}

/// Refuse a card a newer one replaces: its work would redo or undo the newer card's.
fn check_not_superseded(card: &Card) -> Result<(), BoardError> {
    if card.superseded_by.is_empty() {
        return Ok(());
    }
    Err(BoardError::Refused(
        "card is superseded by a newer card: close it, or re-scope it and clear superseded_by"
            .into(),
    ))
}

/// Ids of the cards that wait on a dependency that is not Done. A dependency
/// that is no longer on the board does not hold anything up.
fn waiting_on_deps(cards: &[Card]) -> BTreeSet<Uuid> {
    let open: BTreeSet<Uuid> = cards
        .iter()
        .filter(|c| c.column != Column::DONE)
        .map(|c| c.id)
        .collect();
    cards
        .iter()
        .filter(|c| c.deps.iter().any(|d| open.contains(d)))
        .map(|c| c.id)
        .collect()
}

/// One inbox entry: the escalation, its reason in plain words, and its card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InboxItem {
    /// As BBP reports it.
    #[serde(flatten)]
    pub escalation: Escalation,
    /// The reason in plain words.
    pub label: &'static str,
    /// The card whose execution owns the BBP task.
    pub card: Option<InboxCard>,
}

/// The board card an escalation belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InboxCard {
    /// Card id.
    pub id: Uuid,
    /// Card number.
    pub number: Option<u64>,
    /// Card title.
    pub title: String,
    /// Its project.
    pub project: String,
}

impl InboxCard {
    fn of(c: &Card) -> Self {
        Self {
            id: c.id,
            number: c.number,
            title: c.title.clone(),
            project: c.project.clone(),
        }
    }

    /// `#N "title"`, or the quoted title alone while the card has no number.
    fn label(&self) -> String {
        match self.number {
            Some(n) => format!("#{n} {:?}", self.title),
            None => format!("{:?}", self.title),
        }
    }
}

/// What an inbox action returned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InboxAnswer {
    /// What it would write; nothing was.
    DryRun(DryRun),
    /// What happened.
    Outcome(Outcome),
    /// It could drop acted-on work: the Remediate card, written before BBP was asked.
    /// On a refusal at the first step this send wrote nothing, but another send of the
    /// same decision may have, so the card says to check BBP's log before closing it.
    Remediated {
        /// What happened.
        outcome: Outcome,
        /// The Backlog card to undo or reconcile the work.
        card: InboxCard,
    },
    /// The escalation is closed and a decision sent from here had filed this Remediate
    /// card for it: the answer to a retry, without sending anything to BBP again.
    Recorded {
        /// The Backlog card to undo or reconcile the work.
        card: InboxCard,
    },
}

/// Namespace of the Remediate card ids (UUIDv5).
const REMEDIATION_NS: Uuid = Uuid::from_u128(0x6d69_6c6c_2d72_656d_6564_6961_7465_0595);

/// One exact inbox decision: the escalation, the rev it was read at, and the action.
/// Two decisions on one escalation (Choose A, Choose B) are two records.
struct Decision<'a> {
    escalation: &'a str,
    rev: u64,
    action: &'a Action,
}

impl Decision<'_> {
    /// The id of this decision's Remediate card in `project`. Every other card id is
    /// random, so a card at this id was written by [`Board::inbox_act`] for it.
    fn card_id(&self, project: &str) -> Result<Uuid, BoardError> {
        let action =
            serde_json::to_string(self.action).map_err(|e| BoardError::Storage(e.to_string()))?;
        let key = format!("{project}\n{}\n{}\n{action}", self.escalation, self.rev);
        Ok(Uuid::new_v5(&REMEDIATION_NS, key.as_bytes()))
    }
}

fn inbox_error(e: InboxError) -> BoardError {
    match e {
        InboxError::Host(why) => BoardError::Storage(format!("bbp host: {why}")),
        other => BoardError::Refused(other.to_string()),
    }
}

/// The answerer label BBP records in the op id: 1 to 32 of `[A-Za-z0-9._-]`.
fn answerer(actor: &str) -> String {
    let label: String = actor
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .take(32)
        .collect();
    if label.is_empty() {
        "board".to_owned()
    } else {
        label
    }
}
