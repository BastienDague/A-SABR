use crate::generate_prio_volume_manager;

// With EVL, the delay due to the queue is not taken into account
// and the updates are automatic (we do not "scan" an actual local queue),
// we just reduce the volume available
generate_prio_volume_manager!(EVLManager, false, true, 1, false);
// with priorities (3 levels)
generate_prio_volume_manager!(PEVLManager, false, true, 3, false);
// with priorities (3 levels) and maximum budgets per level
generate_prio_volume_manager!(PBEVLManager, false, true, 3, true);

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

    fn bp1(size: f64) -> Bundle {
        Bundle {
            source: 0,
            destinations: vec![1],
            priority: 1,
            size,
            expiration: 1000.0,
        }
    }

    fn evl() -> EVLManager {
        let mut manager = EVLManager::new(RATE, DELAY);
        manager.try_init(&make_contact_info(C_START, C_END));
        manager
    }

    fn pevl() -> PEVLManager {
        let mut manager = PEVLManager::new(RATE, DELAY);
        manager.try_init(&make_contact_info(C_START, C_END));
        manager
    }

    fn pbevl() -> PBEVLManager {
        let mut manager = PBEVLManager::new(RATE, DELAY, [BUDGET_P0, BUDGET_P1, BUDGET_P2]);
        manager.try_init(&make_contact_info(C_START, C_END));
        manager
    }

    #[test]
    fn tx_start_unaffected_by_queue_occupancy() {
        // EVL ignores queue delay, so tx_start should stay the same
        // even if schedule_tx has already been called multiple times.

        let mut manager = evl();
        let contact = make_contact_info(C_START, C_END);

        let before = manager.dry_run_tx(&contact, C_START, &bp0(1000.0)).unwrap();

        manager
            .schedule_tx(&contact, C_START, &bp0(1000.0))
            .unwrap();
        manager
            .schedule_tx(&contact, C_START, &bp0(1000.0))
            .unwrap();
        manager
            .schedule_tx(&contact, C_START, &bp0(1000.0))
            .unwrap();

        let after = manager.dry_run_tx(&contact, C_START, &bp0(1000.0)).unwrap();

        assert_eq!(
            before.tx_start, after.tx_start,
            "TEST FAILED: EVL tx_start should not be affected by queue occupancy."
        );
    }

    #[test]
    fn schedule_tx_auto_updates_and_can_saturate() {
        // EVL auto updates queue size, so after enough scheduled volume
        // the next bundle should fail.

        let mut manager = evl();
        let contact = make_contact_info(C_START, C_END);

        for _ in 0..10 {
            assert!(
                manager
                    .schedule_tx(&contact, C_START, &bp0(1000.0))
                    .is_some(),
                "TEST FAILED: first 10 bundles should fit exactly in the contact volume."
            );
        }

        assert!(
            manager
                .schedule_tx(&contact, C_START, &bp0(100.0))
                .is_none(),
            "TEST FAILED: EVL should reject extra volume once capacity is full."
        );
    }

    #[test]
    fn priority_queue_is_used_in_pevl() {
        // In PEVL, queue size depends on bundle priority.
        // Scheduling a priority 1 bundle should affect later priority 1 dry runs.

        let mut manager = pevl();
        let contact = make_contact_info(C_START, C_END);

        let before = manager.dry_run_tx(&contact, C_START, &bp1(1000.0)).unwrap();
        manager
            .schedule_tx(&contact, C_START, &bp1(1000.0))
            .unwrap();
        let after = manager.dry_run_tx(&contact, C_START, &bp1(1000.0)).unwrap();

        assert!(
            after.tx_start >= before.tx_start,
            "TEST FAILED: scheduling a priority 1 bundle should not improve tx_start."
        );
    }

    #[test]
    fn budget_blocks_too_large_bundle_in_pbevl() {
        // Budgeted EVL should reject a bundle if it exceeds the allowed budget.

        let manager = pbevl();
        let contact = make_contact_info(C_START, C_END);

        assert!(
            manager
                .dry_run_tx(&contact, C_START, &bp0(20_000.0))
                .is_none(),
            "TEST FAILED: bundle should be rejected when it exceeds the budget."
        );
    }
}
