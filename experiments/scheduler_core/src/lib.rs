#![no_std]
#![forbid(unsafe_code)]
extern crate alloc;

use alloc::collections::{BTreeMap, VecDeque};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TaskId(u64);
impl TaskId {
    pub fn new(value: u64) -> Option<Self> { if value == 0 { None } else { Some(Self(value)) } }
    pub fn get(self) -> u64 { self.0 }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState { Ready, Running, Blocked, Finished }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error { DuplicateTask, UnknownTask, AlreadyRunning, NoRunningTask, NotBlocked }

/// A logical single-runner state machine; it neither executes nor preempts tasks.
#[derive(Default)]
pub struct Scheduler {
    states: BTreeMap<TaskId, TaskState>,
    ready: VecDeque<TaskId>,
    running: Option<TaskId>,
}

impl Scheduler {
    pub fn new() -> Self { Self::default() }
    pub fn state(&self, id: TaskId) -> Option<TaskState> { self.states.get(&id).copied() }
    pub fn running(&self) -> Option<TaskId> { self.running }
    pub fn ready_count(&self) -> usize { self.ready.len() }

    /// IDs are never reused in this instance, including after a task finishes.
    pub fn create(&mut self, id: TaskId) -> Result<(), Error> {
        if self.states.contains_key(&id) { return Err(Error::DuplicateTask); }
        self.states.insert(id, TaskState::Ready);
        self.ready.push_back(id);
        Ok(())
    }

    /// Select the oldest ready task, or None if there is none.
    pub fn dispatch(&mut self) -> Result<Option<TaskId>, Error> {
        if self.running.is_some() { return Err(Error::AlreadyRunning); }
        let Some(id) = self.ready.pop_front() else { return Ok(None) };
        *self.states.get_mut(&id).expect("ready queue invariant") = TaskState::Running;
        self.running = Some(id);
        Ok(Some(id))
    }

    /// A running task voluntarily yields its turn and rejoins the tail.
    pub fn yield_running(&mut self) -> Result<TaskId, Error> {
        let id = self.running.take().ok_or(Error::NoRunningTask)?;
        *self.states.get_mut(&id).expect("running task invariant") = TaskState::Ready;
        self.ready.push_back(id);
        Ok(id)
    }

    /// A running task blocks until explicitly woken.
    pub fn block_running(&mut self) -> Result<TaskId, Error> {
        let id = self.running.take().ok_or(Error::NoRunningTask)?;
        *self.states.get_mut(&id).expect("running task invariant") = TaskState::Blocked;
        Ok(id)
    }

    /// A blocked task rejoins the end of the ready queue.
    pub fn wake(&mut self, id: TaskId) -> Result<(), Error> {
        let state = self.states.get_mut(&id).ok_or(Error::UnknownTask)?;
        if *state != TaskState::Blocked { return Err(Error::NotBlocked); }
        *state = TaskState::Ready;
        self.ready.push_back(id);
        Ok(())
    }

    pub fn finish_running(&mut self) -> Result<TaskId, Error> {
        let id = self.running.take().ok_or(Error::NoRunningTask)?;
        *self.states.get_mut(&id).expect("running task invariant") = TaskState::Finished;
        Ok(id)
    }
}
