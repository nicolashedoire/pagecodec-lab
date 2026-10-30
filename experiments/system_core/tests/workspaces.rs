use system_core_sketch::{merge_three_way, Actor, Document, Origin, Workspaces};

fn doc(pairs: &[(&str, &str)]) -> Document {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn parallel_edits_stay_isolated_and_merge_with_origins() {
    let base = doc(&[("title", "old"), ("notes", "old"), ("unchanged", "same")]);
    let mut spaces = Workspaces::from_base(base.clone());
    spaces
        .edit(Actor::Human)
        .insert("title".into(), "new human title".into());
    spaces
        .edit(Actor::Assistant)
        .insert("notes".into(), "new assistant notes".into());
    assert_eq!(spaces.base(), &base);
    assert_eq!(spaces.view(Actor::Human)["notes"], "old");
    assert_eq!(spaces.view(Actor::Assistant)["title"], "old");
    let result = spaces.merge();
    assert!(result.conflicts.is_empty());
    assert_eq!(result.fields["title"].origin, Origin::Human);
    assert_eq!(result.fields["notes"].origin, Origin::Assistant);
    assert_eq!(result.fields["unchanged"].origin, Origin::Base);
    let complete = result.materialize().unwrap();
    assert_eq!(complete["title"], "new human title");
    assert_eq!(complete["notes"], "new assistant notes");
}

#[test]
fn identical_changes_and_deletions_keep_provenance() {
    let base = doc(&[
        ("deleted", "x"),
        ("same-edit", "old"),
        ("removed-by-human", "present"),
    ]);
    let human = doc(&[("same-edit", "new")]);
    let ai = doc(&[("same-edit", "new"), ("removed-by-human", "present")]);
    let result = merge_three_way(&base, &human, &ai);
    assert!(result.conflicts.is_empty());
    assert_eq!(result.fields["same-edit"].origin, Origin::Both);
    assert_eq!(result.fields["deleted"].origin, Origin::Both);
    assert_eq!(result.fields["deleted"].value, None);
    assert_eq!(result.fields["removed-by-human"].origin, Origin::Human);
    assert_eq!(result.fields["removed-by-human"].value, None);
    assert_eq!(result.materialize().unwrap(), doc(&[("same-edit", "new")]));
}

#[test]
fn collisions_and_delete_vs_edit_retain_both_inputs_without_implicit_resolution() {
    let base = doc(&[("status", "draft"), ("remove-or-change", "v1")]);
    let human = doc(&[("status", "human"), ("created", "human")]);
    let ai = doc(&[
        ("status", "assistant"),
        ("remove-or-change", "v2"),
        ("created", "assistant"),
    ]);
    let result = merge_three_way(&base, &human, &ai);
    assert_eq!(result.conflicts.len(), 3);
    assert_eq!(
        result
            .conflicts
            .iter()
            .map(|c| c.key.as_str())
            .collect::<Vec<_>>(),
        vec!["created", "remove-or-change", "status"]
    );
    let delete = &result.conflicts[1];
    assert_eq!(delete.base.as_deref(), Some("v1"));
    assert_eq!(delete.human, None);
    assert_eq!(delete.assistant.as_deref(), Some("v2"));
    assert!(!result.fields.contains_key("status"));
    assert_eq!(result.materialize(), Err("unresolved conflicts"));
}

#[test]
fn empty_and_missing_keys_are_distinct_from_empty_string() {
    let base = Document::new();
    let human = doc(&[("x", "")]);
    let ai = Document::new();
    let result = merge_three_way(&base, &human, &ai);
    assert_eq!(result.materialize().unwrap(), human);
    assert_eq!(result.fields["x"].origin, Origin::Human);
    assert!(merge_three_way(&base, &base, &base)
        .materialize()
        .unwrap()
        .is_empty());
}

#[test]
fn repeated_merge_is_deterministic_and_does_not_modify_inputs() {
    let base = doc(&[("a", "1"), ("b", "2")]);
    let human = doc(&[("a", "h"), ("b", "2")]);
    let ai = doc(&[("a", "a"), ("b", "3")]);
    let originals = (base.clone(), human.clone(), ai.clone());
    assert_eq!(
        merge_three_way(&base, &human, &ai),
        merge_three_way(&base, &human, &ai)
    );
    assert_eq!((base, human, ai), originals);
}
