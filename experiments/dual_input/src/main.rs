#![no_std]
#![no_main]

extern crate alloc;
use alloc::collections::BTreeSet;
use core::alloc::{GlobalAlloc, Layout};
use core::arch::{asm, global_asm};
use core::fmt::{self, Write};
use core::panic::PanicInfo;
use core::ptr::addr_of_mut;
use core::sync::atomic::{AtomicUsize, Ordering};
use dual_input_lab::{
    parse_agent_line, Actor, AgentCommand, Button, CommandError, Event, Grant, ObjectId, Outcome,
    Point, RouteError, Router, SharedValue, Window, WindowId, WorkspaceId,
};

global_asm!(
    r#"
    .section .text.boot, "ax"
    .global _start
_start:
    msr daifset, #0xf
    mrs x0, CurrentEL
    cmp x0, #4
    b.ne 3f
    mrs x0, SCTLR_EL1
    tbnz x0, #0, 3f
    tbnz x0, #2, 3f
    ldr x0, =__bss_start
    ldr x1, =__bss_end
1:
    cmp x0, x1
    b.hs 2f
    str xzr, [x0], #8
    b 1b
2:
    ldr x0, =__stack_top
    mov sp, x0
    bl rust_main
3:
    wfe
    b 3b
"#
);

const HEAP_SIZE: usize = 128 * 1024;
#[repr(align(4096))]
#[allow(dead_code)]
struct Heap([u8; HEAP_SIZE]);
static mut HEAP: Heap = Heap([0; HEAP_SIZE]);
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Bump;
unsafe impl GlobalAlloc for Bump {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let base = addr_of_mut!(HEAP) as *mut u8 as usize;
        let mut current = NEXT.load(Ordering::Relaxed);
        loop {
            let Some(start) = base
                .checked_add(current)
                .and_then(|x| x.checked_add(layout.align() - 1))
                .map(|x| (x & !(layout.align() - 1)) - base)
            else {
                return core::ptr::null_mut();
            };
            let Some(end) = start.checked_add(layout.size()) else {
                return core::ptr::null_mut();
            };
            if end > HEAP_SIZE {
                return core::ptr::null_mut();
            }
            match NEXT.compare_exchange_weak(current, end, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => return (addr_of_mut!(HEAP) as *mut u8).add(start),
                Err(n) => current = n,
            }
        }
    }
    unsafe fn dealloc(&self, _: *mut u8, _: Layout) { /* one-shot guest */
    }
}
#[global_allocator]
static ALLOC: Bump = Bump;

struct Serial;
impl Write for Serial {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            unsafe {
                while (0x0900_0018 as *const u32).read_volatile() & (1 << 5) != 0 {
                    core::hint::spin_loop();
                }
                (0x0900_0000 as *mut u32).write_volatile(byte as u32);
            }
        }
        Ok(())
    }
}
fn halt() -> ! {
    loop {
        unsafe { asm!("wfe", options(nomem, nostack)) }
    }
}

fn trace(step: &str, router: &Router) {
    let h = router.workspace(WorkspaceId::Human);
    let a = router.workspace(WorkspaceId::Assistant);
    let shared = router.shared(ObjectId(7)).unwrap();
    let _ = writeln!(Serial,
        "TRACE {{\"step\":\"{}\",\"human\":{{\"x\":{},\"y\":{},\"focus\":{},\"revision\":{}}},\"ai\":{{\"x\":{},\"y\":{},\"focus\":{},\"revision\":{}}},\"shared\":{{\"revision\":{},\"value\":{}}}}}",
        step, h.cursor().x, h.cursor().y, h.focus().map_or(0, |id| id.0),
        h.revision(), a.cursor().x, a.cursor().y, a.focus().map_or(0, |id| id.0), a.revision(),
        shared.revision, shared.value);
}

fn grant(actor: Actor, windows: &[WindowId], objects: &[ObjectId]) -> Grant {
    Grant {
        actor,
        workspace: actor.workspace(),
        may_move: true,
        may_focus: true,
        may_click: true,
        windows: windows.iter().copied().collect::<BTreeSet<_>>(),
        writable_objects: objects.iter().copied().collect::<BTreeSet<_>>(),
    }
}

fn move_current(
    r: &mut Router,
    actor: Actor,
    grant: &Grant,
    workspace: WorkspaceId,
    to: Point,
) -> Result<Outcome, RouteError> {
    let expected_workspace_revision = r.workspace(workspace).revision();
    r.route(
        actor,
        grant,
        Event::Move {
            workspace,
            expected_workspace_revision,
            to,
        },
    )
}
fn focus_current(
    r: &mut Router,
    actor: Actor,
    grant: &Grant,
    workspace: WorkspaceId,
    window: WindowId,
) -> Result<Outcome, RouteError> {
    let expected_workspace_revision = r.workspace(workspace).revision();
    r.route(
        actor,
        grant,
        Event::Focus {
            workspace,
            expected_workspace_revision,
            window,
        },
    )
}
fn click_current(
    r: &mut Router,
    actor: Actor,
    grant: &Grant,
    workspace: WorkspaceId,
    window: WindowId,
) -> Result<Outcome, RouteError> {
    let expected_workspace_revision = r.workspace(workspace).revision();
    r.route(
        actor,
        grant,
        Event::Click {
            workspace,
            expected_workspace_revision,
            window,
            button: Button::Primary,
        },
    )
}
fn write_current(
    r: &mut Router,
    actor: Actor,
    grant: &Grant,
    workspace: WorkspaceId,
    window: WindowId,
    expected_revision: u64,
    value: i64,
) -> Result<Outcome, RouteError> {
    let expected_workspace_revision = r.workspace(workspace).revision();
    r.route(
        actor,
        grant,
        Event::WriteShared {
            workspace,
            expected_workspace_revision,
            window,
            object: ObjectId(7),
            expected_revision,
            value,
        },
    )
}

#[no_mangle]
pub extern "C" fn rust_main() -> ! {
    let mut r = Router::new(800, 600).unwrap();
    assert!(r.add_window(
        WorkspaceId::Human,
        Window {
            id: WindowId(11),
            x: 0,
            y: 0,
            width: 300,
            height: 200
        }
    ));
    assert!(r.add_window(
        WorkspaceId::Assistant,
        Window {
            id: WindowId(22),
            x: 0,
            y: 0,
            width: 300,
            height: 200
        }
    ));
    assert!(r.add_window(
        WorkspaceId::Assistant,
        Window {
            id: WindowId(23),
            x: 400,
            y: 0,
            width: 100,
            height: 100
        }
    ));
    assert!(!r.add_window(
        WorkspaceId::Human,
        Window {
            id: WindowId(11),
            x: 1,
            y: 1,
            width: 2,
            height: 2
        }
    ));
    assert!(r.add_shared(ObjectId(7), 100));
    assert!(!r.add_shared(ObjectId(7), 999));
    let human = grant(Actor::Human, &[WindowId(11)], &[ObjectId(7)]);
    let ai = grant(
        Actor::Assistant,
        &[WindowId(22), WindowId(23)],
        &[ObjectId(7)],
    );
    trace("initial", &r);

    let to = Point { x: 25, y: 35 };
    assert_eq!(
        move_current(&mut r, Actor::Human, &human, WorkspaceId::Human, to),
        Ok(Outcome::Moved { to })
    );
    assert_eq!(
        click_current(
            &mut r,
            Actor::Human,
            &human,
            WorkspaceId::Human,
            WindowId(11)
        ),
        Ok(Outcome::Clicked {
            window: WindowId(11),
            at: to
        })
    );
    trace("human_click", &r);
    let ai_point = Point { x: 45, y: 60 };
    assert_eq!(
        parse_agent_line("AI_CLICK 17 PRIMARY\n"),
        Err(CommandError::Unsupported)
    );
    assert_eq!(
        parse_agent_line("AI_MOVE 0 45 60\n"),
        Err(CommandError::InvalidArgument)
    );
    assert_eq!(
        parse_agent_line("AI_MOVE 17 45 60"),
        Err(CommandError::InvalidFrame)
    );
    assert_eq!(
        parse_agent_line("AI_SCROLL 18 1\n"),
        Err(CommandError::Unsupported)
    );
    assert_eq!(
        parse_agent_line("AI_MOVE 17 45 60 extra\n"),
        Err(CommandError::InvalidArgument)
    );
    assert_eq!(
        parse_agent_line("AI_MOVE 17 45 60\nAI_MOVE 18 4 5\n"),
        Err(CommandError::InvalidFrame)
    );
    let negative = parse_agent_line("AI_MOVE 18 -1 60\n").unwrap();
    let negative_event = negative.into_event(r.workspace(WorkspaceId::Assistant).revision());
    assert_eq!(
        r.route(Actor::Assistant, &ai, negative_event),
        Err(RouteError::OutOfBounds)
    );
    let command = parse_agent_line("AI_MOVE 17 45 60\n").unwrap();
    assert_eq!(
        command,
        AgentCommand::Move {
            request_id: 17,
            x: 45,
            y: 60
        }
    );
    let event = command.into_event(r.workspace(WorkspaceId::Assistant).revision());
    assert_eq!(
        r.route(Actor::Assistant, &ai, event),
        Ok(Outcome::Moved { to: ai_point })
    );
    let _ = writeln!(Serial, "AI_ACK 17 MOVED 45 60");
    assert_eq!(
        click_current(
            &mut r,
            Actor::Assistant,
            &ai,
            WorkspaceId::Assistant,
            WindowId(22)
        ),
        Ok(Outcome::Clicked {
            window: WindowId(22),
            at: ai_point
        })
    );
    assert_eq!(r.workspace(WorkspaceId::Human).cursor(), to);
    assert_eq!(r.workspace(WorkspaceId::Human).focus(), Some(WindowId(11)));
    trace("ai_click", &r);

    assert_eq!(
        move_current(
            &mut r,
            Actor::Assistant,
            &ai,
            WorkspaceId::Human,
            Point { x: 50, y: 50 }
        ),
        Err(RouteError::WrongActorOrWorkspace)
    );
    assert_eq!(
        focus_current(&mut r, Actor::Human, &ai, WorkspaceId::Human, WindowId(11)),
        Err(RouteError::WrongActorOrWorkspace)
    );
    assert_eq!(
        focus_current(
            &mut r,
            Actor::Assistant,
            &ai,
            WorkspaceId::Assistant,
            WindowId(11)
        ),
        Err(RouteError::NotGranted)
    );
    assert_eq!(
        move_current(
            &mut r,
            Actor::Human,
            &human,
            WorkspaceId::Human,
            Point { x: -1, y: 0 }
        ),
        Err(RouteError::OutOfBounds)
    );
    assert_eq!(r.workspace(WorkspaceId::Human).cursor(), to);
    trace("cross_scope_denied", &r);

    assert_eq!(
        write_current(
            &mut r,
            Actor::Human,
            &human,
            WorkspaceId::Human,
            WindowId(11),
            0,
            101
        ),
        Ok(Outcome::Written {
            object: ObjectId(7),
            revision: 1
        })
    );
    trace("human_commit", &r);
    assert_eq!(
        write_current(
            &mut r,
            Actor::Assistant,
            &ai,
            WorkspaceId::Assistant,
            WindowId(22),
            0,
            202
        ),
        Err(RouteError::Conflict {
            expected: 0,
            actual: 1
        })
    );
    assert_eq!(
        r.shared(ObjectId(7)),
        Some(SharedValue {
            revision: 1,
            value: 101
        })
    );
    trace("stale_ai_conflict", &r);

    let new_ai_point = Point { x: 420, y: 20 };
    let previous_ai_revision = r.workspace(WorkspaceId::Assistant).revision();
    assert!(move_current(
        &mut r,
        Actor::Assistant,
        &ai,
        WorkspaceId::Assistant,
        new_ai_point
    )
    .is_ok());
    assert_eq!(
        r.route(
            Actor::Assistant,
            &ai,
            Event::Focus {
                workspace: WorkspaceId::Assistant,
                expected_workspace_revision: previous_ai_revision,
                window: WindowId(23)
            }
        ),
        Err(RouteError::StaleWorkspace {
            expected: previous_ai_revision,
            actual: previous_ai_revision + 1
        })
    );
    let mut limited = ai.clone();
    limited.may_focus = false;
    assert_eq!(
        click_current(
            &mut r,
            Actor::Assistant,
            &limited,
            WorkspaceId::Assistant,
            WindowId(23)
        ),
        Err(RouteError::NotGranted)
    );
    assert_eq!(
        r.workspace(WorkspaceId::Assistant).focus(),
        Some(WindowId(22))
    );
    assert_eq!(r.workspace(WorkspaceId::Human).focus(), Some(WindowId(11)));
    trace("focus_denied", &r);
    let _ = writeln!(Serial, "PASS: dual-input prototype");
    halt()
}

#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! {
    let _ = writeln!(Serial, "FAIL: guest assertion");
    halt()
}
