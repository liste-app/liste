//! Hybrid logical clocks (Section 6).
//!
//! An [`Hlc`] is `(wall_ms, counter, device)`. Comparing the three in that
//! order gives a total order over every op ever written, even by devices
//! that were offline, and the order agrees with wall time whenever clocks
//! are close. Each device keeps one [`HlcClock`] that never goes backwards:
//! a wall clock that jumps back only freezes the wall component and grows
//! the counter until real time catches up.

use serde::{Deserialize, Serialize};

use crate::ids::Id;

/// A hybrid logical timestamp. The derived ordering is the total order.
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Hlc {
    /// Milliseconds since the Unix epoch, as seen by the writing device.
    pub wall_ms: u64,
    /// Breaks ties between events in the same millisecond on one device,
    /// and carries causality when a remote clock is ahead.
    pub counter: u32,
    /// The device that produced the timestamp. Breaks the remaining ties.
    pub device: Id,
}

impl std::fmt::Debug for Hlc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Hlc({}:{}:{})", self.wall_ms, self.counter, self.device)
    }
}

/// Source of wall-clock milliseconds. Injected so the web build and tests
/// can supply their own; only native builds get a default.
pub trait WallClock {
    fn now_ms(&self) -> u64;
}

/// The operating system clock.
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
impl WallClock for SystemClock {
    fn now_ms(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}

/// Per-device clock that issues strictly increasing [`Hlc`]s.
#[derive(Debug, Clone)]
pub struct HlcClock<C> {
    device: Id,
    last_wall_ms: u64,
    last_counter: u32,
    wall: C,
}

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
impl HlcClock<SystemClock> {
    /// A clock for `device` driven by the operating system clock.
    pub fn new(device: Id) -> Self {
        HlcClock::with_wall(device, SystemClock)
    }
}

impl<C: WallClock> HlcClock<C> {
    /// A clock for `device` driven by `wall`.
    pub fn with_wall(device: Id, wall: C) -> Self {
        HlcClock {
            device,
            last_wall_ms: 0,
            last_counter: 0,
            wall,
        }
    }

    /// The device this clock stamps for.
    pub fn device(&self) -> Id {
        self.device
    }

    /// Issue a timestamp for a local event. Strictly greater than every
    /// timestamp this clock has issued or observed.
    pub fn tick(&mut self) -> Hlc {
        let now = self.wall.now_ms();
        if now > self.last_wall_ms {
            self.last_wall_ms = now;
            self.last_counter = 0;
        } else {
            self.last_counter += 1;
        }
        self.current()
    }

    /// Fold in a timestamp received from another device so that every
    /// later [`tick`](Self::tick) sorts after it.
    pub fn observe(&mut self, remote: &Hlc) {
        let now = self.wall.now_ms();
        let wall = now.max(self.last_wall_ms).max(remote.wall_ms);
        let counter = if wall == self.last_wall_ms && wall == remote.wall_ms {
            self.last_counter.max(remote.counter) + 1
        } else if wall == self.last_wall_ms {
            self.last_counter + 1
        } else if wall == remote.wall_ms {
            remote.counter + 1
        } else {
            0
        };
        self.last_wall_ms = wall;
        self.last_counter = counter;
    }

    /// The most recent timestamp this clock issued or advanced to.
    pub fn current(&self) -> Hlc {
        Hlc {
            wall_ms: self.last_wall_ms,
            counter: self.last_counter,
            device: self.device,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    #[derive(Clone)]
    struct FakeWall(Rc<Cell<u64>>);

    impl WallClock for FakeWall {
        fn now_ms(&self) -> u64 {
            self.0.get()
        }
    }

    fn clock(device: Id, start: u64) -> (HlcClock<FakeWall>, Rc<Cell<u64>>) {
        let cell = Rc::new(Cell::new(start));
        (HlcClock::with_wall(device, FakeWall(cell.clone())), cell)
    }

    #[test]
    fn ticks_are_strictly_increasing_within_one_millisecond() {
        let (mut c, _) = clock(Id::new(), 1_000);
        let mut prev = c.tick();
        for _ in 0..100 {
            let next = c.tick();
            assert!(next > prev);
            assert_eq!(next.wall_ms, 1_000);
            prev = next;
        }
        assert_eq!(prev.counter, 100);
    }

    #[test]
    fn counter_resets_when_wall_time_advances() {
        let (mut c, wall) = clock(Id::new(), 1_000);
        c.tick();
        c.tick();
        wall.set(1_001);
        let t = c.tick();
        assert_eq!((t.wall_ms, t.counter), (1_001, 0));
    }

    #[test]
    fn a_wall_clock_that_goes_backwards_never_produces_a_smaller_stamp() {
        let (mut c, wall) = clock(Id::new(), 5_000);
        let before = c.tick();
        wall.set(1_000);
        let after = c.tick();
        assert!(after > before);
        assert_eq!(after.wall_ms, 5_000);
        assert_eq!(after.counter, 1);
        wall.set(6_000);
        let later = c.tick();
        assert_eq!((later.wall_ms, later.counter), (6_000, 0));
    }

    #[test]
    fn observing_a_remote_stamp_ahead_of_us_moves_us_past_it() {
        let (mut local, _) = clock(Id::new(), 1_000);
        let remote = Hlc {
            wall_ms: 9_000,
            counter: 3,
            device: Id::new(),
        };
        local.observe(&remote);
        let t = local.tick();
        assert!(t > remote);
        assert_eq!((t.wall_ms, t.counter), (9_000, 5));
    }

    #[test]
    fn observing_a_remote_stamp_behind_us_changes_nothing_but_the_counter() {
        let (mut local, _) = clock(Id::new(), 5_000);
        let before = local.tick();
        let remote = Hlc {
            wall_ms: 1_000,
            counter: 50,
            device: Id::new(),
        };
        local.observe(&remote);
        let after = local.tick();
        assert!(after > before);
        assert_eq!(after.wall_ms, 5_000);
    }

    #[test]
    fn ties_across_devices_are_broken_by_device_id() {
        let a = Id::from_bytes([1; 16]);
        let b = Id::from_bytes([2; 16]);
        let ta = Hlc {
            wall_ms: 1,
            counter: 0,
            device: a,
        };
        let tb = Hlc {
            wall_ms: 1,
            counter: 0,
            device: b,
        };
        assert!(ta < tb);
        assert_ne!(ta, tb);
        let ta2 = Hlc {
            wall_ms: 1,
            counter: 1,
            device: a,
        };
        assert!(ta2 > tb, "counter outranks device id");
    }

    #[test]
    fn two_devices_converge_on_one_order_after_exchanging_stamps() {
        let (mut a, wall_a) = clock(Id::from_bytes([1; 16]), 1_000);
        let (mut b, _) = clock(Id::from_bytes([2; 16]), 1_000);
        let mut all = vec![a.tick(), a.tick(), b.tick()];
        b.observe(&all[1]);
        all.push(b.tick());
        wall_a.set(999); // a's wall clock stalls
        a.observe(&all[3]);
        all.push(a.tick());
        let mut sorted = all.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), all.len(), "every stamp is distinct");
        // Causal pairs sort in causal order; b's first stamp is concurrent
        // with a's and may land anywhere.
        assert!(all[0] < all[1]);
        assert!(all[1] < all[3], "b observed a's stamp before ticking");
        assert!(
            all[3] < all[4],
            "a observed b's stamp despite a stalled wall clock"
        );
    }

    #[test]
    fn json_form_is_stable() {
        let t = Hlc {
            wall_ms: 1_700_000_000_000,
            counter: 2,
            device: Id::from_bytes([0xab; 16]),
        };
        assert_eq!(
            serde_json::to_string(&t).unwrap(),
            r#"{"wall_ms":1700000000000,"counter":2,"device":"abababab-abab-abab-abab-abababababab"}"#
        );
    }
}
