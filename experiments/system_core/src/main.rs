#![forbid(unsafe_code)]

use system_core_sketch::{Actor, Document, Workspaces};

fn main() {
    let mut base = Document::new();
    base.insert("title".into(), "Work plan".into());
    base.insert("notes".into(), "Initial notes".into());
    base.insert("status".into(), "draft".into());
    let mut workspaces = Workspaces::from_base(base);
    workspaces
        .edit(Actor::Human)
        .insert("title".into(), "Human title".into());
    workspaces
        .edit(Actor::Assistant)
        .insert("notes".into(), "Assistant notes".into());
    workspaces
        .edit(Actor::Human)
        .insert("status".into(), "review".into());
    workspaces
        .edit(Actor::Assistant)
        .insert("status".into(), "ready".into());
    let merged = workspaces.merge();
    println!("Demonstration only: independent workspace snapshots\n");
    for (key, field) in &merged.fields {
        println!("{key}: {:?} (origin: {:?})", field.value, field.origin);
    }
    for conflict in &merged.conflicts {
        println!(
            "CONFLICT {}: base={:?}, human={:?}, assistant={:?}",
            conflict.key, conflict.base, conflict.human, conflict.assistant
        );
    }
    println!("No AI was invoked; conflicts are not automatically resolved.");
}
