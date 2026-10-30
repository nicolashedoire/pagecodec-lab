//! User-space concepts for a future system. This is not an operating system or a security boundary.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;

/// A deliberately small structured document. A field is either a string or absent.
pub type Document = BTreeMap<String, String>;

/// The actor which authored changes in its own workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Actor {
    Human,
    Assistant,
}

/// Two snapshots independently derived from the same immutable base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspaces {
    base: Document,
    human: Document,
    assistant: Document,
}

impl Workspaces {
    pub fn from_base(base: Document) -> Self {
        Self {
            human: base.clone(),
            assistant: base.clone(),
            base,
        }
    }

    pub fn base(&self) -> &Document {
        &self.base
    }

    pub fn view(&self, actor: Actor) -> &Document {
        match actor {
            Actor::Human => &self.human,
            Actor::Assistant => &self.assistant,
        }
    }

    /// Mutations change only the selected copy; this does not enforce real OS permissions.
    pub fn edit(&mut self, actor: Actor) -> &mut Document {
        match actor {
            Actor::Human => &mut self.human,
            Actor::Assistant => &mut self.assistant,
        }
    }

    pub fn merge(&self) -> MergeResult {
        merge_three_way(&self.base, &self.human, &self.assistant)
    }
}

/// Attribution describes net changes from the common base, not full edit history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Base,
    Human,
    Assistant,
    Both,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergedField {
    /// None means the field was deleted. Tombstones preserve deletion provenance.
    pub value: Option<String>,
    pub origin: Origin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub key: String,
    pub base: Option<String>,
    pub human: Option<String>,
    pub assistant: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeResult {
    /// Non-conflicting fields and tombstones only, ordered by key.
    pub fields: BTreeMap<String, MergedField>,
    /// Conflicting fields are omitted from `fields` and retained in full here.
    pub conflicts: Vec<Conflict>,
}

impl MergeResult {
    /// Never claim a complete document while any conflicting field is unresolved.
    pub fn materialize(&self) -> Result<Document, &'static str> {
        if !self.conflicts.is_empty() {
            return Err("unresolved conflicts");
        }
        Ok(self
            .fields
            .iter()
            .filter_map(|(k, v)| v.value.as_ref().map(|s| (k.clone(), s.clone())))
            .collect())
    }
}

/// Deterministic field-level three-way merge. It does not interpret document semantics.
pub fn merge_three_way(base: &Document, human: &Document, assistant: &Document) -> MergeResult {
    let keys: BTreeSet<&String> = base
        .keys()
        .chain(human.keys())
        .chain(assistant.keys())
        .collect();
    let mut fields = BTreeMap::new();
    let mut conflicts = Vec::new();
    for key in keys {
        let b = base.get(key);
        let h = human.get(key);
        let a = assistant.get(key);
        let decision = if h == b && a == b {
            Some((b, Origin::Base))
        } else if h == b {
            Some((a, Origin::Assistant))
        } else if a == b {
            Some((h, Origin::Human))
        } else if h == a {
            Some((h, Origin::Both))
        } else {
            None
        };
        match decision {
            Some((value, origin)) => {
                fields.insert(
                    key.clone(),
                    MergedField {
                        value: value.cloned(),
                        origin,
                    },
                );
            }
            None => conflicts.push(Conflict {
                key: key.clone(),
                base: b.cloned(),
                human: h.cloned(),
                assistant: a.cloned(),
            }),
        }
    }
    MergeResult { fields, conflicts }
}

/// A future advisor can suggest a resolution; the caller must validate and authorize it.
/// No implementation, model, MCP client, or automatic conflict resolution is provided.
pub trait DecisionAdvisor {
    fn suggest(&self, conflict: &Conflict) -> Option<Resolution>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    pub key: String,
    pub value: Option<String>,
    pub explanation: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Read,
    Write,
    Merge,
}

/// A seam for future OS-enforced authorization. Calling a trait is not authentication.
pub trait CapabilityBroker {
    fn allowed(&self, actor: Actor, operation: Operation, resource: &str) -> bool;
}
