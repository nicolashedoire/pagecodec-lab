//! Simulated typed input adapter for an assistant workspace. Not an MCP server,
//! input driver, security boundary, or proof of independent operating-system focus.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;
use alloc::vec::Vec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowId(pub u64);

/// Opaque only in this illustration: callers can construct it, so this is not a credential.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Workspace {
    Human,
    Assistant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

/// Half-open coordinate bounds: left/top included, right/bottom excluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}
impl Bounds {
    pub fn valid(self) -> bool {
        self.left < self.right && self.top < self.bottom
    }
    pub fn contains(self, p: Point) -> bool {
        self.valid() && p.x >= self.left && p.x < self.right && p.y >= self.top && p.y < self.bottom
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Primary,
    Secondary,
    Middle,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonState {
    Pressed,
    Released,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerAction {
    Move { position: Point },
    Button { button: Button, state: ButtonState },
    Scroll { dx: i32, dy: i32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    Move,
    Button,
    Scroll,
}
impl PointerAction {
    pub fn kind(self) -> ActionKind {
        match self {
            Self::Move { .. } => ActionKind::Move,
            Self::Button { .. } => ActionKind::Button,
            Self::Scroll { .. } => ActionKind::Scroll,
        }
    }
}

/// A structured request a future MCP layer could decode, not an actual MCP wire schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentCommand {
    pub request_id: u64,
    pub capability: CapabilityId,
    pub workspace: Workspace,
    pub target: WindowId,
    pub expected_revision: u64,
    pub action: PointerAction,
}

/// Snapshot must be provided by a trusted routing layer; its origin is not validated here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointerSnapshot {
    pub workspace: Workspace,
    pub target: WindowId,
    pub revision: u64,
    pub bounds: Bounds,
    pub current_position: Point,
}

/// Simulated grant only. A caller-provided value can be forged; the future OS must
/// authenticate principals, issue protected capabilities and enforce them itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimulatedGrant {
    pub capability: CapabilityId,
    pub workspace: Workspace,
    pub windows: Vec<WindowId>,
    pub actions: Vec<ActionKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectReason {
    NotAssistantWorkspace,
    CapabilityMismatch,
    TargetMismatch,
    TargetNotGranted,
    ActionNotGranted,
    StaleRevision,
    InvalidBounds,
    PointerOutsideTarget,
    ScrollOutOfRange,
}

/// Events produced here are always tagged Assistant; this tag alone confers no authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointerEvent {
    pub actor: Workspace,
    pub target: WindowId,
    pub expected_revision: u64,
    pub position: Point,
    pub action: PointerAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Accepted(PointerEvent),
    Rejected(RejectReason),
}

/// Request and snapshot are the exact caller-supplied values used for validation.
/// They are not authenticated evidence of real OS state or permission. Outcome is
/// only this adapter's proposal/rejection, not a dispatched event or OS response.
/// Trace is returned data, not durable, tamper-proof or access-controlled logging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceRecord {
    /// Full typed input, including requested position/button/scroll where present.
    pub command: AgentCommand,
    /// Full snapshot provided to the check; it does not prove the state was trusted.
    pub snapshot: PointerSnapshot,
    pub request_id: u64,
    pub target: WindowId,
    pub expected_revision: u64,
    pub observed_revision: u64,
    pub action: ActionKind,
    pub outcome: Outcome,
}

/// Converts a structured request into a proposed event. Pure: no system actions,
/// IO or network; caller must atomically revalidate state and authorization when
/// dispatching the event to avoid time-of-check/time-of-use races.
pub fn adapt(
    command: AgentCommand,
    snapshot: PointerSnapshot,
    grant: &SimulatedGrant,
) -> TraceRecord {
    let result = check(command, snapshot, grant);
    TraceRecord {
        command,
        snapshot,
        request_id: command.request_id,
        target: command.target,
        expected_revision: command.expected_revision,
        observed_revision: snapshot.revision,
        action: command.action.kind(),
        outcome: match result {
            Ok(event) => Outcome::Accepted(event),
            Err(reason) => Outcome::Rejected(reason),
        },
    }
}

fn check(
    command: AgentCommand,
    snapshot: PointerSnapshot,
    grant: &SimulatedGrant,
) -> Result<PointerEvent, RejectReason> {
    if command.workspace != Workspace::Assistant
        || snapshot.workspace != Workspace::Assistant
        || grant.workspace != Workspace::Assistant
    {
        return Err(RejectReason::NotAssistantWorkspace);
    }
    if command.capability != grant.capability {
        return Err(RejectReason::CapabilityMismatch);
    }
    if command.target != snapshot.target {
        return Err(RejectReason::TargetMismatch);
    }
    if !grant.windows.contains(&command.target) {
        return Err(RejectReason::TargetNotGranted);
    }
    if !grant.actions.contains(&command.action.kind()) {
        return Err(RejectReason::ActionNotGranted);
    }
    if command.expected_revision != snapshot.revision {
        return Err(RejectReason::StaleRevision);
    }
    if !snapshot.bounds.valid() {
        return Err(RejectReason::InvalidBounds);
    }
    let position = match command.action {
        PointerAction::Move { position } => position,
        PointerAction::Button { .. } | PointerAction::Scroll { .. } => snapshot.current_position,
    };
    if !snapshot.bounds.contains(position) {
        return Err(RejectReason::PointerOutsideTarget);
    }
    if let PointerAction::Scroll { dx, dy } = command.action {
        if dx.unsigned_abs() > 1200 || dy.unsigned_abs() > 1200 {
            return Err(RejectReason::ScrollOutOfRange);
        }
    }
    Ok(PointerEvent {
        actor: Workspace::Assistant,
        target: command.target,
        expected_revision: command.expected_revision,
        position,
        action: command.action,
    })
}
