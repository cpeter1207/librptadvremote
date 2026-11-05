use crate::{scan::Scanner, *};

fn list(members: &[u32], priority: Option<u32>) -> ScanList {
    ScanList {
        selector: 1,
        name: "local".into(),
        members: members.to_vec(),
        settle_ms: 100,
        priority,
        ..Default::default()
    }
}
fn scanner(members: &[u32], priority: Option<u32>) -> Scanner {
    Scanner::new(
        list(members, priority),
        ScanDirection::Up,
        None,
        RequestId(42),
    )
    .unwrap()
}

#[test]
fn scan_contract_rejects_overflowing_dwell_and_preserves_monotonic_deadlines() {
    for duration in [0, u64::MAX] {
        assert!(matches!(
            Scanner::new(
                list(&[1, 2, 3], None),
                ScanDirection::Up,
                Some(duration),
                RequestId(1)
            ),
            Err(Error::InvalidConfiguration)
        ));
    }
    for (direction, first, after_skip) in [(ScanDirection::Up, 1, 3), (ScanDirection::Down, 3, 1)] {
        let mut scan =
            Scanner::new(list(&[1, 2, 3], None), direction, Some(250), RequestId(42)).unwrap();
        assert_eq!(scan.snapshot().dwell_ms, 250);
        assert_eq!(scan.due(), Some(first));
        let completed = scan.complete(first, 10, Ok(()), None).unwrap();
        assert_eq!(completed.id, RequestId(42));
        assert_eq!(completed.result, Ok(()));
        assert_eq!(scan.snapshot().next_deadline_ms, Some(360));
        scan.activity(359, false);
        assert_eq!(scan.due(), None);
        scan.activity(360, false);
        assert_eq!(scan.due(), Some(2));
        assert!(scan.complete(2, 360, Ok(()), None).is_none());
        scan.skip();
        assert_eq!(scan.snapshot().skipped, 1);
        assert_eq!(scan.due(), Some(after_skip));
    }
    let mut scan = scanner(&[1, 2], None);
    assert_eq!(scan.dwell(250), Ok(()));
    scan.complete(1, 10, Ok(()), None);
    assert_eq!(scan.snapshot().next_deadline_ms, Some(360));
    assert_eq!(scan.dwell(200), Ok(()));
    assert_eq!(scan.snapshot().next_deadline_ms, Some(310));
    for invalid in [0, u64::MAX] {
        assert_eq!(scan.dwell(invalid), Err(Error::InvalidConfiguration));
    }
    scan.resume_delay(20);
    scan.activity(100, true);
    assert!(scan.snapshot().activity_hold);
    assert_eq!(scan.due(), None);
    scan.activity(110, false);
    assert!(scan.snapshot().activity_hold);
    scan.activity(129, false);
    assert_eq!(scan.due(), None);
    scan.activity(310, false);
    assert!(!scan.snapshot().activity_hold);
    assert_eq!(scan.due(), Some(2));
    scan.direction(ScanDirection::Down);
    assert_eq!(scan.snapshot().direction, ScanDirection::Down);
}

#[test]
fn scan_contract_priority_rotation_skips_failed_members_without_repeated_tunes() {
    let mut scan = scanner(&[1, 2, 3], Some(2));
    for (now, member) in [(0, 1), (600, 2), (1200, 3), (1800, 2), (2400, 1)] {
        scan.activity(now, false);
        assert_eq!(scan.due(), Some(member));
        scan.complete(member, now, Ok(()), None);
    }
    assert_eq!(scan.priority(Some(9)), Err(Error::NotFound));
    assert_eq!(scan.priority(None), Ok(()));
    assert_eq!(scan.snapshot().priority, None);
    assert_eq!(scan.priority(Some(1)), Ok(()));
    scan.activity(3000, false);
    assert_eq!(scan.due(), Some(2));
    scan.complete(2, 3000, Err(Error::Backend), Some(-99));
    assert_eq!(scan.snapshot().last_error, Some(-99));
    assert_eq!(scan.snapshot().failed_memory, Some(2));
    assert_eq!(scan.priority(Some(2)), Ok(()));
    assert_eq!(scan.due(), Some(3));
    scan.complete(3, 3000, Ok(()), None);
    scan.activity(3600, false);
    assert_eq!(scan.due(), Some(1));
    let mut one = scanner(&[1], Some(1));
    one.complete(1, 0, Ok(()), None);
    one.activity(600, false);
    assert_eq!(one.due(), None);
    assert_eq!(one.snapshot().current_memory, Some(1));
    assert_eq!(one.snapshot().next_deadline_ms, Some(1100));
}

#[test]
fn scan_contract_pause_and_failures_complete_start_requests_once() {
    let mut scan = scanner(&[1, 2, 3], None);
    let paused = scan.pause().unwrap();
    assert_eq!(paused.id, RequestId(42));
    assert_eq!(paused.result, Err(Error::Stale));
    assert!(scan.snapshot().paused);
    assert_eq!(scan.snapshot().next_deadline_ms, None);
    assert_eq!(scan.due(), None);
    assert!(scan.pause().is_none());
    assert_eq!(scan.resume(), Ok(()));
    scan.skip(); // No member has yet been applied.
    assert_eq!(scan.due(), Some(2));
    let mut scan = scanner(&[1, 2, 3], None);
    for (member, cause) in [(1, None), (2, Some(-99)), (3, None)] {
        assert_eq!(scan.due(), Some(member));
        let completion = scan.complete(member, 0, Err(Error::Backend), cause);
        if member == 3 {
            let completion = completion.unwrap();
            assert_eq!(completion.id, RequestId(42));
            assert_eq!(completion.result, Err(Error::Backend));
        } else {
            assert!(completion.is_none());
        }
    }
    assert!(!scan.snapshot().active);
    assert_eq!(scan.snapshot().skipped, 3);
    assert_eq!(scan.resume(), Err(Error::NotFound));
    assert_eq!(scan.due(), None);
    assert!(scan.stop().is_none());
    let mut scan = scanner(&[1], None);
    let stopped = scan.stop().unwrap();
    assert_eq!(stopped.id, RequestId(42));
    assert_eq!(stopped.result, Err(Error::Stale));
}
