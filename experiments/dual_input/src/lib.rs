//! Prototype data model only. Caller-supplied grants are not authentication.
#![no_std]
#![forbid(unsafe_code)]
extern crate alloc;

use alloc::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Actor {
    Human,
    Assistant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum WorkspaceId {
    Human,
    Assistant,
}

impl Actor {
    pub fn workspace(self) -> WorkspaceId {
        match self {
            Self::Human => WorkspaceId::Human,
            Self::Assistant => WorkspaceId::Assistant,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct WindowId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ObjectId(pub u64);

/// Only a complete primary click is represented. Press/release and scroll are
/// unsupported and require separate future event types, not silent coercion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Primary,
}

/// One explicitly supported wire command. It is a parser only, not an MCP
/// server, identity check, authenticated channel, or guest serial reader.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentCommand {
    Move { request_id: u64, x: i32, y: i32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandError {
    InvalidFrame,
    InvalidArgument,
    Unsupported,
}

/// Parse a complete single ASCII line: `AI_MOVE <id> <x> <y>\n`.
/// Clicks, button-down/up and scrolling remain unsupported on this wire.
pub fn parse_agent_line(line: &str) -> Result<AgentCommand, CommandError> {
    if line.len() > 96
        || !line.is_ascii()
        || !line.ends_with('\n')
        || line[..line.len() - 1].contains('\n')
    {
        return Err(CommandError::InvalidFrame);
    }
    let mut words = line.split_ascii_whitespace();
    let Some(name) = words.next() else {
        return Err(CommandError::InvalidFrame);
    };
    if name != "AI_MOVE" {
        return Err(CommandError::Unsupported);
    }
    let id = words
        .next()
        .ok_or(CommandError::InvalidArgument)?
        .parse::<u64>()
        .map_err(|_| CommandError::InvalidArgument)?;
    let x = words
        .next()
        .ok_or(CommandError::InvalidArgument)?
        .parse::<i32>()
        .map_err(|_| CommandError::InvalidArgument)?;
    let y = words
        .next()
        .ok_or(CommandError::InvalidArgument)?
        .parse::<i32>()
        .map_err(|_| CommandError::InvalidArgument)?;
    if id == 0 || words.next().is_some() {
        return Err(CommandError::InvalidArgument);
    }
    Ok(AgentCommand::Move {
        request_id: id,
        x,
        y,
    })
}

impl AgentCommand {
    /// Caller supplies a fresh snapshot revision; Router rechecks it on dispatch.
    pub fn into_event(self, expected_workspace_revision: u64) -> Event {
        match self {
            Self::Move { x, y, .. } => Event::Move {
                workspace: WorkspaceId::Assistant,
                expected_workspace_revision,
                to: Point { x, y },
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    pub id: WindowId,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Window {
    fn contains(&self, p: Point) -> bool {
        let x = i64::from(p.x) - i64::from(self.x);
        let y = i64::from(p.y) - i64::from(self.y);
        x >= 0 && y >= 0 && x < i64::from(self.width) && y < i64::from(self.height)
    }
}

/// Test-only policy description; values are constructed by the caller and
/// cannot establish a trustworthy identity, permission, or OS boundary.
#[derive(Clone, Debug)]
pub struct Grant {
    pub actor: Actor,
    pub workspace: WorkspaceId,
    pub may_move: bool,
    pub may_focus: bool,
    pub may_click: bool,
    pub windows: BTreeSet<WindowId>,
    pub writable_objects: BTreeSet<ObjectId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Move {
        workspace: WorkspaceId,
        expected_workspace_revision: u64,
        to: Point,
    },
    Focus {
        workspace: WorkspaceId,
        expected_workspace_revision: u64,
        window: WindowId,
    },
    Click {
        workspace: WorkspaceId,
        expected_workspace_revision: u64,
        window: WindowId,
        button: Button,
    },
    WriteShared {
        workspace: WorkspaceId,
        window: WindowId,
        object: ObjectId,
        expected_workspace_revision: u64,
        expected_revision: u64,
        value: i64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Moved { to: Point },
    Focused { window: WindowId },
    Clicked { window: WindowId, at: Point },
    Written { object: ObjectId, revision: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteError {
    WrongActorOrWorkspace,
    NotGranted,
    OutOfBounds,
    UnknownWindow,
    PointerOutsideWindow,
    WindowNotFocused,
    UnknownObject,
    Conflict { expected: u64, actual: u64 },
    StaleWorkspace { expected: u64, actual: u64 },
    RevisionExhausted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SharedValue {
    pub revision: u64,
    pub value: i64,
}

#[derive(Clone, Debug)]
pub struct Workspace {
    cursor: Point,
    focus: Option<WindowId>,
    revision: u64,
    windows: BTreeMap<WindowId, Window>,
}

impl Workspace {
    fn new() -> Self {
        Self {
            cursor: Point { x: 0, y: 0 },
            focus: None,
            revision: 0,
            windows: BTreeMap::new(),
        }
    }
    pub fn cursor(&self) -> Point {
        self.cursor
    }
    pub fn focus(&self) -> Option<WindowId> {
        self.focus
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
}

pub struct Router {
    width: u32,
    height: u32,
    human: Workspace,
    assistant: Workspace,
    shared: BTreeMap<ObjectId, SharedValue>,
}

impl Router {
    pub fn new(width: u32, height: u32) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        Some(Self {
            width,
            height,
            human: Workspace::new(),
            assistant: Workspace::new(),
            shared: BTreeMap::new(),
        })
    }
    pub fn workspace(&self, id: WorkspaceId) -> &Workspace {
        match id {
            WorkspaceId::Human => &self.human,
            WorkspaceId::Assistant => &self.assistant,
        }
    }
    fn workspace_mut(&mut self, id: WorkspaceId) -> &mut Workspace {
        match id {
            WorkspaceId::Human => &mut self.human,
            WorkspaceId::Assistant => &mut self.assistant,
        }
    }
    /// Bootstrap only; does not model real OS window-creation permissions.
    pub fn add_window(&mut self, id: WorkspaceId, window: Window) -> bool {
        if window.id.0 == 0
            || window.width == 0
            || window.height == 0
            || self.workspace(id).windows.contains_key(&window.id)
        {
            return false;
        }
        let ws = self.workspace_mut(id);
        let Some(next) = ws.revision.checked_add(1) else {
            return false;
        };
        ws.windows.insert(window.id, window);
        ws.revision = next;
        true
    }
    /// Bootstrap only; reject accidental replacement of an existing object.
    pub fn add_shared(&mut self, id: ObjectId, value: i64) -> bool {
        if self.shared.contains_key(&id) {
            return false;
        }
        self.shared.insert(id, SharedValue { revision: 0, value });
        true
    }
    pub fn shared(&self, id: ObjectId) -> Option<SharedValue> {
        self.shared.get(&id).copied()
    }

    pub fn route(
        &mut self,
        actor: Actor,
        grant: &Grant,
        event: Event,
    ) -> Result<Outcome, RouteError> {
        let target = match event {
            Event::Move { workspace, .. }
            | Event::Focus { workspace, .. }
            | Event::Click { workspace, .. }
            | Event::WriteShared { workspace, .. } => workspace,
        };
        if actor != grant.actor || actor.workspace() != target || grant.workspace != target {
            return Err(RouteError::WrongActorOrWorkspace);
        }
        let expected_workspace_revision = match event {
            Event::Move {
                expected_workspace_revision,
                ..
            }
            | Event::Focus {
                expected_workspace_revision,
                ..
            }
            | Event::Click {
                expected_workspace_revision,
                ..
            }
            | Event::WriteShared {
                expected_workspace_revision,
                ..
            } => expected_workspace_revision,
        };
        let actual = self.workspace(target).revision;
        if expected_workspace_revision != actual {
            return Err(RouteError::StaleWorkspace {
                expected: expected_workspace_revision,
                actual,
            });
        }
        let Some(next_workspace_revision) = actual.checked_add(1) else {
            return Err(RouteError::RevisionExhausted);
        };
        match event {
            Event::Move { to, .. } => {
                if !grant.may_move {
                    return Err(RouteError::NotGranted);
                }
                if to.x < 0 || to.y < 0 || to.x as u32 >= self.width || to.y as u32 >= self.height {
                    return Err(RouteError::OutOfBounds);
                }
                let ws = self.workspace_mut(target);
                ws.cursor = to;
                ws.revision = next_workspace_revision;
                Ok(Outcome::Moved { to })
            }
            Event::Focus { window, .. } => {
                if !grant.may_focus || !grant.windows.contains(&window) {
                    return Err(RouteError::NotGranted);
                }
                if !self.workspace(target).windows.contains_key(&window) {
                    return Err(RouteError::UnknownWindow);
                }
                let ws = self.workspace_mut(target);
                ws.focus = Some(window);
                ws.revision = next_workspace_revision;
                Ok(Outcome::Focused { window })
            }
            Event::Click {
                window,
                button: Button::Primary,
                ..
            } => {
                if !grant.may_click || !grant.windows.contains(&window) {
                    return Err(RouteError::NotGranted);
                }
                let ws = self.workspace(target);
                let Some(target_window) = ws.windows.get(&window) else {
                    return Err(RouteError::UnknownWindow);
                };
                let at = ws.cursor;
                if !target_window.contains(at) {
                    return Err(RouteError::PointerOutsideWindow);
                }
                if ws.focus != Some(window) && !grant.may_focus {
                    return Err(RouteError::NotGranted);
                }
                let ws = self.workspace_mut(target);
                ws.focus = Some(window);
                ws.revision = next_workspace_revision;
                Ok(Outcome::Clicked { window, at })
            }
            Event::WriteShared {
                window,
                object,
                expected_revision,
                value,
                ..
            } => {
                if !grant.windows.contains(&window) || !grant.writable_objects.contains(&object) {
                    return Err(RouteError::NotGranted);
                }
                let ws = self.workspace(target);
                if !ws.windows.contains_key(&window) {
                    return Err(RouteError::UnknownWindow);
                }
                if ws.focus != Some(window) {
                    return Err(RouteError::WindowNotFocused);
                }
                let Some(old) = self.shared.get_mut(&object) else {
                    return Err(RouteError::UnknownObject);
                };
                if old.revision != expected_revision {
                    return Err(RouteError::Conflict {
                        expected: expected_revision,
                        actual: old.revision,
                    });
                }
                let Some(revision) = old.revision.checked_add(1) else {
                    return Err(RouteError::RevisionExhausted);
                };
                *old = SharedValue { revision, value };
                self.workspace_mut(target).revision = next_workspace_revision;
                Ok(Outcome::Written { object, revision })
            }
        }
    }
}
