//! Synthetic multi-actor input model. Caller-supplied IDs are not credentials.
#![no_std]
#![forbid(unsafe_code)]
extern crate alloc;

use alloc::{collections::BTreeMap, vec::Vec};
use actor_core_sketch::{ActorId, ActorKind, ActorState, Error as RegistryError, Position, Registry, SpaceId};

/// Workspace-local, half-open bounds: left/top included, right/bottom excluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds { pub left: i32, pub top: i32, pub right: i32, pub bottom: i32 }
impl Bounds {
    pub fn valid(self) -> bool { self.left < self.right && self.top < self.bottom }
    pub fn contains(self, p: Position) -> bool {
        self.valid() && p.x >= self.left && p.x < self.right && p.y >= self.top && p.y < self.bottom
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupError { InvalidBounds, Registry(RegistryError) }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveError { UnknownSpace, UnknownActor, WrongSpace, OutOfBounds, Registry(RegistryError) }

/// Caller-supplied labels, positions and outcomes are a logical trace, not an
/// authenticated or tamper-resistant audit log. Contains no timestamp or I/O.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveTrace {
    pub actor: ActorId, pub space: SpaceId, pub position: Position, pub outcome: Result<(), MoveError>,
}

/// One serialized in-memory router. The public API has no fixed actor count;
/// capacity and trace retention depend on the allocator and available memory.
#[derive(Default)]
pub struct Router {
    registry: Registry,
    bounds: BTreeMap<SpaceId, Bounds>,
    trace: Vec<MoveTrace>,
}
impl Router {
    pub fn new() -> Self { Self::default() }
    pub fn create_space(&mut self, space: SpaceId, bounds: Bounds) -> Result<(), SetupError> {
        if !bounds.valid() { return Err(SetupError::InvalidBounds); }
        self.registry.create_space(space.clone()).map_err(SetupError::Registry)?;
        self.bounds.insert(space, bounds);
        Ok(())
    }
    pub fn create_actor(&mut self, actor: ActorId, kind: ActorKind) -> Result<(), SetupError> {
        self.registry.create_actor(actor, kind).map_err(SetupError::Registry)
    }
    pub fn assign(&mut self, actor: &ActorId, space: &SpaceId) -> Result<(), SetupError> {
        self.registry.assign(actor, space).map_err(SetupError::Registry)
    }
    pub fn actor(&self, actor: &ActorId) -> Option<&ActorState> { self.registry.actor(actor) }
    pub fn actor_count(&self) -> usize { self.registry.actor_count() }
    pub fn traces(&self) -> &[MoveTrace] { &self.trace }

    /// Check registered actor, workspace assignment and workspace-local bounds;
    /// log both successful and rejected attempts. No caller identity is verified.
    pub fn move_pointer(&mut self, actor: &ActorId, space: &SpaceId, position: Position) -> Result<(), MoveError> {
        let outcome = match (self.bounds.get(space), self.registry.actor(actor)) {
            (None, _) => Err(MoveError::UnknownSpace),
            (_, None) => Err(MoveError::UnknownActor),
            (Some(_), Some(a)) if a.space() != Some(space) => Err(MoveError::WrongSpace),
            (Some(bounds), Some(_)) if !bounds.contains(position) => Err(MoveError::OutOfBounds),
            (Some(_), Some(_)) => self.registry.move_pointer(actor, space, position).map_err(MoveError::Registry),
        };
        self.trace.push(MoveTrace { actor: actor.clone(), space: space.clone(), position, outcome });
        outcome
    }
}
