//! Monotonic, owner-serviced scanning through ordinary validated tune transactions.

use crate::{Completion, Error, RequestId, ScanList};

/// Live scan traversal direction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum ScanDirection {
    /// Reverse configured member order.
    Down = 0,
    /// Follow configured member order.
    #[default]
    Up = 1,
}

/// Typed scan operations; memory/list definitions remain configuration-only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanRequest {
    /// Start one list, optionally selecting a configured speed preset atomically.
    Start {
        /// Configured list selector.
        selector: u32,
        /// Traversal direction.
        direction: ScanDirection,
        /// Live dwell override, or the list's configured value.
        dwell_ms: Option<u64>,
    },
    /// Hold the current member and cancel unsubmitted automatic tuning.
    Pause,
    /// Resume a paused scan after ordinary quiet-time rules.
    Resume,
    /// Stop, leaving the last fully applied member selected.
    Stop,
    /// Change live traversal direction.
    Direction(ScanDirection),
    /// Skip the current member until a new scan is started.
    Skip,
    /// Change live dwell duration.
    SetDwell(u64),
    /// Change live post-activity quiet hold.
    SetResumeDelay(u64),
    /// Change live priority member; absent disables priority excursions.
    SetPriority(Option<u32>),
}

/// Copied scan status; no physical calls or dynamic allocation are required.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScanSnapshot {
    /// Configured list selector.
    pub list: u32,
    /// Last fully applied scan member, if any.
    pub current_memory: Option<u32>,
    /// Scan remains running, including manual/activity holds.
    pub active: bool,
    /// Operator pause is active.
    pub paused: bool,
    /// Qualified activity or its quiet hold prevents another step.
    pub activity_hold: bool,
    /// Live traversal direction.
    pub direction: ScanDirection,
    /// Next eligible monotonic deadline, if a dwell/quiet hold exists.
    pub next_deadline_ms: Option<u64>,
    /// Live dwell duration.
    pub dwell_ms: u64,
    /// Live quiet duration.
    pub resume_delay_ms: u64,
    /// Current priority member.
    pub priority: Option<u32>,
    /// Count of skipped positions in this scan run.
    pub skipped: usize,
    /// Most recent failed member, retained after later successful steps.
    pub failed_memory: Option<u32>,
    /// Original cause of that member's failed tune.
    pub last_error: Option<i32>,
}

pub(crate) struct Scanner {
    list: ScanList,
    direction: ScanDirection,
    cursor: usize,
    priority_visit: bool,
    skipped: Vec<bool>,
    active: bool,
    paused: bool,
    current: Option<u32>,
    next: Option<u32>,
    deadline: Option<u64>,
    busy: bool,
    quiet_since: Option<u64>,
    now: u64,
    last_error: Option<i32>,
    failed_memory: Option<u32>,
    pub(crate) start_request: Option<RequestId>,
}

impl Scanner {
    pub(crate) fn new(
        mut list: ScanList,
        direction: ScanDirection,
        dwell: Option<u64>,
        id: RequestId,
    ) -> Result<Self, Error> {
        if let Some(dwell) = dwell {
            if dwell == 0 || dwell.checked_add(list.settle_ms).is_none() {
                return Err(Error::InvalidConfiguration);
            }
            list.dwell_ms = dwell;
        }
        let cursor = if direction == ScanDirection::Up {
            0
        } else {
            list.members.len() - 1
        };
        let next = Some(list.members[cursor]);
        Ok(Self {
            skipped: vec![false; list.members.len()],
            list,
            direction,
            cursor,
            priority_visit: false,
            active: true,
            paused: false,
            current: None,
            next,
            deadline: None,
            busy: false,
            quiet_since: None,
            now: 0,
            last_error: None,
            failed_memory: None,
            start_request: Some(id),
        })
    }

    pub(crate) fn snapshot(&self) -> ScanSnapshot {
        let quiet_deadline = self
            .quiet_since
            .map(|time| time.saturating_add(self.list.resume_delay_ms));
        ScanSnapshot {
            list: self.list.selector,
            current_memory: self.current,
            active: self.active,
            paused: self.paused,
            activity_hold: self.busy || quiet_deadline.is_some_and(|time| time > self.now),
            direction: self.direction,
            next_deadline_ms: if self.active && !self.paused {
                self.deadline.into_iter().chain(quiet_deadline).max()
            } else {
                None
            },
            dwell_ms: self.list.dwell_ms,
            resume_delay_ms: self.list.resume_delay_ms,
            priority: self.list.priority,
            skipped: self.skipped.iter().filter(|skipped| **skipped).count(),
            failed_memory: self.failed_memory,
            last_error: self.last_error,
        }
    }

    pub(crate) fn activity(&mut self, now: u64, busy: bool) {
        self.now = now;
        if busy {
            self.quiet_since = None;
        } else if self.busy {
            self.quiet_since = Some(now);
        }
        self.busy = busy;
    }

    pub(crate) fn due(&mut self) -> Option<u32> {
        if !self.active
            || self.paused
            || self.busy
            || self
                .quiet_since
                .is_some_and(|time| time.saturating_add(self.list.resume_delay_ms) > self.now)
        {
            return None;
        }
        if let Some(next) = self.next {
            return Some(next);
        }
        if self.deadline.is_some_and(|time| time > self.now) {
            return None;
        }
        let next = self.choose_next()?;
        if Some(next) == self.current {
            self.deadline = Some(self.now.saturating_add(self.list.dwell_ms));
            return None;
        }
        self.next = Some(next);
        Some(next)
    }

    fn choose_next(&mut self) -> Option<u32> {
        if !self.priority_visit {
            if let Some(priority) = self.list.priority {
                if Some(priority) != self.current
                    && self
                        .list
                        .members
                        .iter()
                        .zip(&self.skipped)
                        .any(|(member, skipped)| *member == priority && !skipped)
                {
                    self.priority_visit = true;
                    return Some(priority);
                }
            }
        }
        self.priority_visit = false;
        let ordinary_available = self
            .list
            .members
            .iter()
            .zip(&self.skipped)
            .any(|(member, skipped)| !skipped && Some(*member) != self.list.priority);
        for _ in 0..self.list.members.len() {
            self.cursor = match self.direction {
                ScanDirection::Up => {
                    if self.cursor == self.list.members.len() - 1 {
                        0
                    } else {
                        self.cursor + 1
                    }
                }
                ScanDirection::Down => {
                    if self.cursor == 0 {
                        self.list.members.len() - 1
                    } else {
                        self.cursor - 1
                    }
                }
            };
            if !self.skipped[self.cursor]
                && (!ordinary_available
                    || Some(self.list.members[self.cursor]) != self.list.priority)
            {
                return Some(self.list.members[self.cursor]);
            }
        }
        self.active = false;
        None
    }

    pub(crate) fn complete(
        &mut self,
        member: u32,
        now: u64,
        result: Result<(), Error>,
        cause: Option<i32>,
    ) -> Option<Completion> {
        self.now = now;
        self.next = None;
        if let Err(error) = result {
            self.last_error = Some(cause.unwrap_or(error as i32));
            self.failed_memory = Some(member);
            for (index, configured) in self.list.members.iter().enumerate() {
                if *configured == member {
                    self.skipped[index] = true;
                }
            }
            self.next = self.choose_next();
            self.deadline = None;
        } else {
            self.current = Some(member);
            self.deadline = Some(
                now.saturating_add(self.list.settle_ms)
                    .saturating_add(self.list.dwell_ms),
            );
        }
        if result.is_ok() || !self.active {
            self.start_request
                .take()
                .map(|id| Completion { id, result })
        } else {
            None
        }
    }

    pub(crate) fn stop(&mut self) -> Option<Completion> {
        self.active = false;
        self.paused = false;
        self.next = None;
        self.start_request.take().map(|id| Completion {
            id,
            result: Err(Error::Stale),
        })
    }

    pub(crate) fn pause(&mut self) -> Option<Completion> {
        self.paused = true;
        self.start_request.take().map(|id| Completion {
            id,
            result: Err(Error::Stale),
        })
    }

    pub(crate) fn resume(&mut self) -> Result<(), Error> {
        if !self.active {
            return Err(Error::NotFound);
        }
        self.paused = false;
        Ok(())
    }

    pub(crate) fn direction(&mut self, direction: ScanDirection) {
        self.direction = direction;
    }

    pub(crate) fn skip(&mut self) {
        if let Some(current) = self.current {
            for (index, member) in self.list.members.iter().enumerate() {
                if *member == current {
                    self.skipped[index] = true;
                }
            }
        }
        self.next = self.choose_next();
        self.deadline = None;
    }

    pub(crate) fn dwell(&mut self, duration: u64) -> Result<(), Error> {
        if duration == 0 || duration.checked_add(self.list.settle_ms).is_none() {
            return Err(Error::InvalidConfiguration);
        }
        self.deadline = self.deadline.map(|deadline| {
            deadline
                .saturating_sub(self.list.dwell_ms)
                .saturating_add(duration)
        });
        self.list.dwell_ms = duration;
        Ok(())
    }

    pub(crate) fn resume_delay(&mut self, duration: u64) {
        self.list.resume_delay_ms = duration;
    }

    pub(crate) fn priority(&mut self, member: Option<u32>) -> Result<(), Error> {
        if member.is_some_and(|member| !self.list.members.contains(&member)) {
            return Err(Error::NotFound);
        }
        self.list.priority = member;
        Ok(())
    }
}
