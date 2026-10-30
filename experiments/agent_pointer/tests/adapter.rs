use agent_pointer_contract::*;

fn sample() -> (AgentCommand, PointerSnapshot, SimulatedGrant) {
    let window = WindowId(7);
    let cmd = AgentCommand {
        request_id: 42,
        capability: CapabilityId(8),
        workspace: Workspace::Assistant,
        target: window,
        expected_revision: 2,
        action: PointerAction::Move {
            position: Point { x: 10, y: 20 },
        },
    };
    let state = PointerSnapshot {
        workspace: Workspace::Assistant,
        target: window,
        revision: 2,
        bounds: Bounds {
            left: 0,
            top: 0,
            right: 100,
            bottom: 100,
        },
        current_position: Point { x: 0, y: 0 },
    };
    let grant = SimulatedGrant {
        capability: CapabilityId(8),
        workspace: Workspace::Assistant,
        windows: vec![window],
        actions: vec![ActionKind::Move, ActionKind::Button, ActionKind::Scroll],
    };
    (cmd, state, grant)
}

#[test]
fn allowed_move_is_tagged_assistant_and_traced() {
    let (cmd, state, grant) = sample();
    let record = adapt(cmd, state, &grant);
    assert_eq!(record.command, cmd);
    assert_eq!(record.snapshot, state);
    assert_eq!(record.request_id, 42);
    assert_eq!(record.action, ActionKind::Move);
    assert_eq!(record.expected_revision, 2);
    assert_eq!(record.observed_revision, 2);
    let Outcome::Accepted(event) = record.outcome else {
        panic!("expected accepted")
    };
    assert_eq!(event.actor, Workspace::Assistant);
    assert_eq!(event.target, WindowId(7));
    assert_eq!(event.position, Point { x: 10, y: 20 });
}

#[test]
fn denied_action_keeps_exact_request_and_snapshot_in_trace() {
    let (mut cmd, mut state, mut grant) = sample();
    cmd.action = PointerAction::Button {
        button: Button::Secondary,
        state: ButtonState::Released,
    };
    state.current_position = Point { x: 22, y: 33 };
    grant.actions = vec![ActionKind::Move];
    let record = adapt(cmd, state, &grant);
    assert_eq!(record.command, cmd);
    assert_eq!(record.snapshot, state);
    assert_eq!(record.action, ActionKind::Button);
    assert_eq!(
        record.outcome,
        Outcome::Rejected(RejectReason::ActionNotGranted)
    );
}

#[test]
fn accepted_scroll_keeps_requested_deltas_and_proposed_position() {
    let (mut cmd, mut state, grant) = sample();
    cmd.action = PointerAction::Scroll { dx: -17, dy: 42 };
    state.current_position = Point { x: 22, y: 33 };
    let record = adapt(cmd, state, &grant);
    assert_eq!(record.command, cmd);
    assert_eq!(record.snapshot, state);
    let Outcome::Accepted(event) = record.outcome else {
        panic!("expected accepted")
    };
    assert_eq!(event.action, cmd.action);
    assert_eq!(event.position, state.current_position);
}

#[test]
fn human_workspace_is_rejected_no_matter_which_input_claims_it() {
    let (mut cmd, state, grant) = sample();
    cmd.workspace = Workspace::Human;
    assert_eq!(
        adapt(cmd, state, &grant).outcome,
        Outcome::Rejected(RejectReason::NotAssistantWorkspace)
    );
    let (cmd, mut state, grant) = sample();
    state.workspace = Workspace::Human;
    assert_eq!(
        adapt(cmd, state, &grant).outcome,
        Outcome::Rejected(RejectReason::NotAssistantWorkspace)
    );
    let (cmd, state, mut grant) = sample();
    grant.workspace = Workspace::Human;
    assert_eq!(
        adapt(cmd, state, &grant).outcome,
        Outcome::Rejected(RejectReason::NotAssistantWorkspace)
    );
}

#[test]
fn stale_and_mismatched_grants_are_rejected() {
    let (mut cmd, state, grant) = sample();
    cmd.expected_revision = 1;
    assert_eq!(
        adapt(cmd, state, &grant).outcome,
        Outcome::Rejected(RejectReason::StaleRevision)
    );
    cmd.expected_revision = 3;
    assert_eq!(
        adapt(cmd, state, &grant).outcome,
        Outcome::Rejected(RejectReason::StaleRevision)
    );
    cmd.expected_revision = 2;
    cmd.capability = CapabilityId(9);
    assert_eq!(
        adapt(cmd, state, &grant).outcome,
        Outcome::Rejected(RejectReason::CapabilityMismatch)
    );
}

#[test]
fn ungranted_window_wrong_target_and_action_are_rejected() {
    let (mut cmd, mut state, grant) = sample();
    cmd.target = WindowId(9);
    assert_eq!(
        adapt(cmd, state, &grant).outcome,
        Outcome::Rejected(RejectReason::TargetMismatch)
    );
    state.target = WindowId(9);
    assert_eq!(
        adapt(cmd, state, &grant).outcome,
        Outcome::Rejected(RejectReason::TargetNotGranted)
    );
    let (mut cmd, state, mut grant) = sample();
    cmd.action = PointerAction::Button {
        button: Button::Primary,
        state: ButtonState::Pressed,
    };
    grant.actions = vec![ActionKind::Move];
    assert_eq!(
        adapt(cmd, state, &grant).outcome,
        Outcome::Rejected(RejectReason::ActionNotGranted)
    );
}

#[test]
fn target_bounds_are_half_open_and_button_uses_current_position() {
    let (mut cmd, mut state, grant) = sample();
    cmd.action = PointerAction::Move {
        position: Point { x: 100, y: 0 },
    };
    assert_eq!(
        adapt(cmd, state, &grant).outcome,
        Outcome::Rejected(RejectReason::PointerOutsideTarget)
    );
    cmd.action = PointerAction::Button {
        button: Button::Primary,
        state: ButtonState::Pressed,
    };
    state.current_position = Point { x: -1, y: 20 };
    assert_eq!(
        adapt(cmd, state, &grant).outcome,
        Outcome::Rejected(RejectReason::PointerOutsideTarget)
    );
    state.current_position = Point { x: 99, y: 99 };
    assert!(matches!(
        adapt(cmd, state, &grant).outcome,
        Outcome::Accepted(_)
    ));
    state.bounds.right = 0;
    assert_eq!(
        adapt(cmd, state, &grant).outcome,
        Outcome::Rejected(RejectReason::InvalidBounds)
    );
}

#[test]
fn scroll_limits_handle_signed_extremes() {
    let (mut cmd, state, grant) = sample();
    cmd.action = PointerAction::Scroll {
        dx: i32::MIN,
        dy: 0,
    };
    assert_eq!(
        adapt(cmd, state, &grant).outcome,
        Outcome::Rejected(RejectReason::ScrollOutOfRange)
    );
    cmd.action = PointerAction::Scroll {
        dx: -1200,
        dy: 1200,
    };
    assert!(matches!(
        adapt(cmd, state, &grant).outcome,
        Outcome::Accepted(_)
    ));
}
