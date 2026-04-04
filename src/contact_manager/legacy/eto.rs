use crate::generate_prio_volume_manager;

// With ETO the delay due to the queue is taken into account (from the current time)
// and the updates are not automatic, the queue is expected to be modified by
// external means
generate_prio_volume_manager!(ETOManager, true, false, 1, false);
// with priorities (3 levels)
generate_prio_volume_manager!(PETOManager, true, false, 3, false);
// with priorities (3 levels) and maximum budgets per level
generate_prio_volume_manager!(PBETOManager, true, false, 3, true);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{bundle::Bundle, contact::ContactInfo, contact_manager::ContactManager};

    const RATE: f64 = 1000.0;
    const DELAY: f64 = 4.0;
    const C_START: f64 = 0.0;
    const C_END: f64 = 10.0;
    const BUDGET_P0: f64 = 10_000.0;
    const BUDGET_P1: f64 = 10_000.0;
    const BUDGET_P2: f64 = 10_000.0;

    fn make_contact_info(start: f64, end: f64) -> ContactInfo {
        ContactInfo::new(0, 1, start, end)
    }

    fn bp0(size: f64) -> Bundle {
        Bundle {
            source: 0,
            destinations: vec![1],
            priority: 0,
            size,
            expiration: 1000.0,
        }
    }

    fn eto() -> ETOManager {
        let mut manager = ETOManager::new(RATE, DELAY);
        manager.try_init(&make_contact_info(C_START, C_END));
        manager
    }

    fn pbeto() -> PBETOManager {
        let mut manager = PBETOManager::new(RATE, DELAY, [BUDGET_P0, BUDGET_P1, BUDGET_P2]);
        manager.try_init(&make_contact_info(C_START, C_END));
        manager
    }

    #[test]
    fn schedule_tx_does_not_consume_volume() {
        // ETO does not auto update the queue, so calling schedule_tx many times
        // should still keep working.

        let mut manager = eto();
        let contact = make_contact_info(C_START, C_END);

        for i in 0..20 {
            assert!(
                manager
                    .schedule_tx(&contact, C_START, &bp0(1000.0))
                    .is_some(),
                "TEST FAILED: ETO schedule_tx should never saturate (call {}).",
                i + 1
            );
        }
    }

    #[test]
    fn schedule_tx_always_returns_same_result() {
        // Since the queue is not updated automatically in ETO,
        // two identical calls should return the same result.

        let mut manager = eto();
        let contact = make_contact_info(C_START, C_END);
        let bundle = bp0(1000.0);

        let first = manager.schedule_tx(&contact, C_START, &bundle);
        let second = manager.schedule_tx(&contact, C_START, &bundle);

        assert_eq!(first.is_some(), second.is_some());

        if let (Some(a), Some(b)) = (first, second) {
            assert_eq!(a.tx_start, b.tx_start);
            assert_eq!(a.tx_end, b.tx_end);
            assert_eq!(a.expiration, b.expiration);
            assert_eq!(a.rx_start, b.rx_start);
            assert_eq!(a.rx_end, b.rx_end);
        }
    }

    #[test]
    fn budget_blocks_too_large_bundle() {
        // If the bundle is larger than the available budget, it should fail.

        let manager = pbeto();
        let contact = make_contact_info(C_START, C_END);

        assert!(
            manager
                .dry_run_tx(&contact, C_START, &bp0(20_000.0))
                .is_none(),
            "TEST FAILED: bundle should be rejected when it exceeds the budget."
        );
    }

    #[cfg(feature = "manual_queueing")]
    #[test]
    fn manual_enqueue_shifts_tx_start_from_at_time() {
        // With manual queueing, adding volume in queue should shift tx_start.

        let mut manager = eto();
        let contact = make_contact_info(C_START, C_END);

        manager.manual_enqueue(&bp0(2000.0));

        let data = manager.dry_run_tx(&contact, 3.0, &bp0(100.0)).unwrap();

        assert_eq!(
            data.tx_start, 5.0,
            "TEST FAILED: tx_start should be at_time + queue/rate for ETO."
        );
    }

    #[cfg(feature = "manual_queueing")]
    #[test]
    fn manual_enqueue_shift_can_push_past_contact_end() {
        // If the manual queue is too large, the next bundle should not fit anymore.

        let mut manager = eto();
        let contact = make_contact_info(C_START, C_END);

        manager.manual_enqueue(&bp0(9900.0));

        assert!(
            manager.dry_run_tx(&contact, C_START, &bp0(200.0)).is_none(),
            "TEST FAILED: Bundle should not fit when manual queue shift pushes tx_end past contact end."
        );
    }
}
