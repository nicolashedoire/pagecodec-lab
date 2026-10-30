#![no_std]
#![forbid(unsafe_code)]
extern crate alloc;

use alloc::{string::String, vec::Vec};
use actor_core_sketch::{ActorId, ActorKind, Registry, SpaceId};
use system_core_sketch::{Actor, Document, MergeResult, Origin, Workspaces};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error { SameActor, SameSpace, UnknownActor, WrongKind, WrongSpace, UnboundActor }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeEvent {
    pub actor: ActorId,
    pub space: SpaceId,
    pub key: String,
    pub value: Option<String>,
}

/// Two explicitly bound copies. IDs are caller-supplied labels, not credentials.
pub struct Bridge {
    registry: Registry,
    human: ActorId,
    assistant: ActorId,
    human_space: SpaceId,
    assistant_space: SpaceId,
    copies: Workspaces,
    events: Vec<ChangeEvent>,
}

impl Bridge {
    pub fn new(registry: Registry, human: ActorId, human_space: SpaceId,
               assistant: ActorId, assistant_space: SpaceId, base: Document) -> Result<Self, Error> {
        if human == assistant { return Err(Error::SameActor); }
        if human_space == assistant_space { return Err(Error::SameSpace); }
        for (id, space, kind) in [(&human, &human_space, ActorKind::Human),
                                   (&assistant, &assistant_space, ActorKind::Assistant)] {
            let actor = registry.actor(id).ok_or(Error::UnknownActor)?;
            if actor.kind() != kind { return Err(Error::WrongKind); }
            if actor.space() != Some(space) { return Err(Error::WrongSpace); }
        }
        Ok(Self { registry, human, assistant, human_space, assistant_space,
                  copies: Workspaces::from_base(base), events: Vec::new() })
    }

    fn role(&self, actor: &ActorId, space: &SpaceId) -> Result<Actor, Error> {
        let (role, expected) = if actor == &self.human { (Actor::Human, &self.human_space) }
            else if actor == &self.assistant { (Actor::Assistant, &self.assistant_space) }
            else { return Err(Error::UnboundActor) };
        let state = self.registry.actor(actor).ok_or(Error::UnknownActor)?;
        if expected != space || state.space() != Some(space) { return Err(Error::WrongSpace); }
        Ok(role)
    }

    pub fn edit(&mut self, actor: &ActorId, space: &SpaceId, key: &str, value: Option<&str>) -> Result<(), Error> {
        let role = self.role(actor, space)?;
        match value {
            Some(v) => { self.copies.edit(role).insert(String::from(key), String::from(v)); }
            None => { self.copies.edit(role).remove(key); }
        }
        self.events.push(ChangeEvent { actor: actor.clone(), space: space.clone(), key: String::from(key), value: value.map(String::from) });
        Ok(())
    }
    pub fn view(&self, actor: &ActorId, space: &SpaceId) -> Result<&Document, Error> {
        Ok(self.copies.view(self.role(actor, space)?))
    }
    pub fn merge(&self) -> MergeResult { self.copies.merge() }
    pub fn events(&self) -> &[ChangeEvent] { &self.events }
    /// Resolves a net-change provenance label to the IDs bound to this bridge.
    pub fn provenance(&self, origin: Origin) -> (Option<&ActorId>, Option<&ActorId>) {
        match origin {
            Origin::Base => (None, None), Origin::Human => (Some(&self.human), None),
            Origin::Assistant => (None, Some(&self.assistant)),
            Origin::Both => (Some(&self.human), Some(&self.assistant)),
        }
    }
}
