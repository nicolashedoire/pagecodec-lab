#![no_std]
extern crate alloc;

use alloc::{collections::{BTreeMap, BTreeSet}, string::String, vec::Vec};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdError { Empty, InvalidCharacter }

fn validate_id(value: &str) -> Result<(), IdError> {
    if value.is_empty() { return Err(IdError::Empty); }
    if !value.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_') {
        return Err(IdError::InvalidCharacter);
    }
    Ok(())
}

macro_rules! id_type {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
        pub struct $name(String);
        impl $name {
            pub fn new(value: &str) -> Result<Self, IdError> {
                validate_id(value)?;
                Ok(Self(String::from(value)))
            }
            pub fn as_str(&self) -> &str { &self.0 }
        }
    };
}
id_type!(ActorId);
id_type!(SpaceId);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorKind { Human, Assistant, Remote }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position { pub x: i32, pub y: i32 }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorState { kind: ActorKind, space: Option<SpaceId>, pointer: Option<Position> }
impl ActorState {
    pub fn kind(&self) -> ActorKind { self.kind }
    pub fn space(&self) -> Option<&SpaceId> { self.space.as_ref() }
    pub fn pointer(&self) -> Option<Position> { self.pointer }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    SpaceCreated { space: SpaceId },
    ActorCreated { actor: ActorId, kind: ActorKind },
    ActorAssigned { actor: ActorId, space: SpaceId },
    PointerMoved { actor: ActorId, space: SpaceId, position: Position },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error { DuplicateSpace, DuplicateActor, UnknownSpace, UnknownActor, WrongSpace }

/// Logical state only. IDs are labels, not authenticated capabilities.
#[derive(Default)]
pub struct Registry {
    spaces: BTreeSet<SpaceId>,
    actors: BTreeMap<ActorId, ActorState>,
    events: Vec<Event>,
}

impl Registry {
    pub fn new() -> Self { Self::default() }
    pub fn actor(&self, id: &ActorId) -> Option<&ActorState> { self.actors.get(id) }
    pub fn actor_count(&self) -> usize { self.actors.len() }
    pub fn space_count(&self) -> usize { self.spaces.len() }
    pub fn events(&self) -> &[Event] { &self.events }

    pub fn create_space(&mut self, space: SpaceId) -> Result<(), Error> {
        if !self.spaces.insert(space.clone()) { return Err(Error::DuplicateSpace); }
        self.events.push(Event::SpaceCreated { space });
        Ok(())
    }
    pub fn create_actor(&mut self, actor: ActorId, kind: ActorKind) -> Result<(), Error> {
        if self.actors.contains_key(&actor) { return Err(Error::DuplicateActor); }
        self.actors.insert(actor.clone(), ActorState { kind, space: None, pointer: None });
        self.events.push(Event::ActorCreated { actor, kind });
        Ok(())
    }
    pub fn assign(&mut self, actor: &ActorId, space: &SpaceId) -> Result<(), Error> {
        if !self.spaces.contains(space) { return Err(Error::UnknownSpace); }
        let state = self.actors.get_mut(actor).ok_or(Error::UnknownActor)?;
        // A cursor position from another coordinate space must never be reused.
        state.pointer = None;
        state.space = Some(space.clone());
        self.events.push(Event::ActorAssigned { actor: actor.clone(), space: space.clone() });
        Ok(())
    }
    /// Update only state; callers provide any recording policy. No movement
    /// event is appended here, so callers must not claim an audit history.
    pub fn move_pointer_unlogged(&mut self, actor: &ActorId, space: &SpaceId, position: Position) -> Result<(), Error> {
        if !self.spaces.contains(space) { return Err(Error::UnknownSpace); }
        let state = self.actors.get_mut(actor).ok_or(Error::UnknownActor)?;
        if state.space.as_ref() != Some(space) { return Err(Error::WrongSpace); }
        state.pointer = Some(position);
        Ok(())
    }
    pub fn move_pointer(&mut self, actor: &ActorId, space: &SpaceId, position: Position) -> Result<(), Error> {
        self.move_pointer_unlogged(actor,space,position)?;
        self.events.push(Event::PointerMoved { actor: actor.clone(), space: space.clone(), position });
        Ok(())
    }
}
