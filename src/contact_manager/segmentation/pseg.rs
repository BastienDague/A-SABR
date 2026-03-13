#[cfg(feature = "first_depleted")]
use crate::types::Volume;
use crate::{
    bundle::Bundle,
    contact::ContactInfo,
    contact_manager::{
        ContactManager, ContactManagerTxData,
        segmentation::{BaseSegmentationManager, Segment},
    },
    parsing::{Lexer, Parser, ParsingState},
    types::{DataRate, Date, Duration, Priority},
};

/// Priority-aware segmentation manager. Tracks bandwidth availability per priority level
/// using booking intervals.
#[cfg_attr(feature = "debug", derive(Debug))]
pub struct PSegmentationManager {
    /// A list of segments tracking the priority level booked for each time interval.
    booking: Vec<Segment<Priority>>,
    /// A list of segments representing different data rates during contact intervals.
    rate_intervals: Vec<Segment<DataRate>>,
    /// A list of segments representing delay times associated with different intervals.
    delay_intervals: Vec<Segment<Duration>>,
    #[cfg(feature = "first_depleted")]
    /// The total volume at initialization.
    original_volume: Volume,
}

impl PSegmentationManager {
    pub fn new(
        rate_intervals: Vec<Segment<DataRate>>,
        delay_intervals: Vec<Segment<Duration>>,
    ) -> Self {
        let booking = Vec::new();

        Self {
            booking,
            rate_intervals,
            delay_intervals,
            #[cfg(feature = "first_depleted")]
            original_volume: 0.0,
        }
    }
}

impl BaseSegmentationManager for PSegmentationManager {
    fn new(
        rate_intervals: Vec<Segment<DataRate>>,
        delay_intervals: Vec<Segment<Duration>>,
    ) -> Self {
        Self::new(rate_intervals, delay_intervals)
    }
}

impl ContactManager for PSegmentationManager {
    /// Simulates the transmission of a bundle based on the contact data and bundle priority.
    ///
    /// # Arguments
    ///
    /// * `_contact_data` - Reference to the contact information (unused in this implementation).
    /// * `at_time` - The current time for scheduling purposes.
    /// * `bundle` - The bundle to be transmitted.
    ///
    /// # Returns
    ///
    /// Optionally returns `ContactManagerTxData` with transmission start and end times, or `None` if the bundle can't be transmitted.
    fn dry_run_tx(
        &self,
        contact_data: &ContactInfo,
        at_time: Date,
        bundle: &Bundle,
    ) -> Option<ContactManagerTxData> {
        let mut tx_start = at_time;
        let mut tx_end_opt: Option<Date> = None;

        for seg in &self.booking {
            // Allows to advance to the first valid segment
            if seg.end <= at_time {
                continue;
            }

            // Segment is not valid, we need to reset the building process with the next segment
            if bundle.priority <= seg.val {
                tx_end_opt = None;
                continue;
            }
            // Start building or pursue ?
            match tx_end_opt {
                // Try to pursue the build process
                Some(tx_end) => {
                    // the seg is valid, check if this is the last one to consider
                    if tx_end < seg.end {
                        let delay = super::get_delay(tx_end, &self.delay_intervals);
                        return Some(ContactManagerTxData {
                            tx_start,
                            tx_end,
                            delay,
                            expiration: seg.end,
                            arrival: tx_end + delay,
                        });
                    }
                    // if we reach this point, the seg is valid, but transmission didn't reach terminaison, check next
                }
                // (re)-start the build process
                None => {
                    tx_start = Date::max(seg.start, at_time);
                    // In most cases, there should be a single rate seg
                    if let Some(tx_end) = super::get_tx_end(
                        &self.rate_intervals,
                        tx_start,
                        bundle.size,
                        contact_data.end,
                    ) {
                        if tx_end < seg.end {
                            let delay = super::get_delay(tx_end, &self.delay_intervals);
                            return Some(ContactManagerTxData {
                                tx_start,
                                tx_end,
                                delay,
                                expiration: seg.end,
                                arrival: tx_end + delay,
                            });
                        }
                        tx_end_opt = Some(tx_end);
                    };
                }
            }
        }
        None
    }

    /// Schedule the transmission of a bundle by updating the booking intervals with the bundle's priority.
    ///
    /// This method shall be called after a dry run ! Implementations might not ensure a clean behavior otherwise.
    ///
    /// # Arguments
    ///
    /// * `_contact_data` - Reference to the contact information (unused in this implementation).
    /// * `at_time` - The current time for scheduling purposes.
    /// * `bundle` - The bundle to be transmitted.
    ///
    /// # Returns
    ///
    /// Optionally returns `ContactManagerTxData` with transmission start and end times, or `None` if the bundle can't be transmitted.
    fn schedule_tx(
        &mut self,
        contact_data: &ContactInfo,
        at_time: Date,
        bundle: &Bundle,
    ) -> Option<ContactManagerTxData> {
        let out = self.dry_run_tx(contact_data, at_time, bundle)?;
        let tx_start = out.tx_start;
        let tx_end = out.tx_end;

        let mut i = 0;
        while i < self.booking.len() {
            let seg = &self.booking[i];

            // Segment completely before tx window
            if seg.end <= tx_start {
                i += 1;
                continue;
            }

            // Segment completely after tx window
            if seg.start >= tx_end {
                break;
            }

            let old_prio = seg.val;

            // Cut before
            if seg.start < tx_start {
                let left = Segment {
                    start: seg.start,
                    end: tx_start,
                    val: old_prio,
                };
                self.booking.insert(i, left);
                self.booking[i + 1].start = tx_start;
                i += 1;
            }

            // Cut after
            if self.booking[i].end > tx_end {
                let right = Segment {
                    start: tx_end,
                    end: self.booking[i].end,
                    val: old_prio,
                };
                self.booking.insert(i + 1, right);
                self.booking[i].end = tx_end;
            }

            // Assign TX priority
            self.booking[i].val = bundle.priority;
            i += 1;
        }

        Some(out)
    }

    /// For first depleted compatibility
    ///
    /// # Returns
    ///
    /// Returns the maximum volume the contact had at initialization.
    #[cfg(feature = "first_depleted")]
    fn get_original_volume(&self) -> Volume {
        self.original_volume
    }

    /// Initializes the segmentation manager by checking that rate and delay intervals have no gaps.
    ///
    /// # Arguments
    ///
    /// * `contact_data` - Reference to the contact information.
    ///
    /// # Returns
    ///
    /// Returns `true` if initialization is successful, or `false` if there are gaps in the intervals.
    fn try_init(&mut self, contact_data: &ContactInfo) -> bool {
        super::try_init(
            &self.rate_intervals,
            &self.delay_intervals,
            &mut self.booking,
            -1,
            #[cfg(feature = "first_depleted")]
            &mut self.original_volume,
            contact_data,
        )
    }
}

/// Implements the `Parser` trait for `PSegmentationManager`, allowing the manager to be parsed from a lexer.
impl Parser<PSegmentationManager> for PSegmentationManager {
    /// Parses a `PSegmentationManager` from the lexer, extracting the rate and delay intervals.
    ///
    /// # Arguments
    ///
    /// * `lexer` - The lexer used for parsing tokens.
    ///
    /// # Returns
    ///
    /// Returns a `ParsingState` indicating whether parsing was successful (`Finished`) or encountered an error (`Error`).
    fn parse(lexer: &mut dyn Lexer) -> ParsingState<PSegmentationManager> {
        super::parse::<PSegmentationManager>(lexer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        bundle::Bundle,
        contact::ContactInfo,
        contact_manager::ContactManager,
        contact_manager::segmentation::BaseSegmentationManager,
    };

    #[test]
    fn test_new_manager() {
        // We create simple segments for rate and delay.
        let rate_intervals = vec![Segment {
            start: 0.0,
            end: 10.0,
            val: 2.0,
        }];

        let delay_intervals = vec![Segment {
            start: 0.0,
            end: 10.0,
            val: 1.0,
        }];

        // Create the priority segmentation manager
        let manager = PSegmentationManager::new(rate_intervals, delay_intervals);

        // When the manager is created, booking should be empty
        assert!(manager.booking.is_empty());

        // Check that the rate intervals were stored correctly
        assert_eq!(manager.rate_intervals.len(), 1);
        assert_eq!(manager.rate_intervals[0].start, 0.0);
        assert_eq!(manager.rate_intervals[0].end, 10.0);
        assert_eq!(manager.rate_intervals[0].val, 2.0);

        // Check that the delay intervals were stored correctly
        assert_eq!(manager.delay_intervals.len(), 1);
        assert_eq!(manager.delay_intervals[0].start, 0.0);
        assert_eq!(manager.delay_intervals[0].end, 10.0);
        assert_eq!(manager.delay_intervals[0].val, 1.0);
    }

    #[test]
    fn test_manager_initial_state() {
        // This test checks the initial state of the manager after creation.

        let rate_intervals = vec![Segment {
            start: 0.0,
            end: 10.0,
            val: 1.0,
        }];

        let delay_intervals = vec![Segment {
            start: 0.0,
            end: 10.0,
            val: 1.0,
        }];

        let manager = PSegmentationManager::new(rate_intervals, delay_intervals);

        // The manager should start with no booking intervals
        assert!(manager.booking.is_empty());
    }

    #[test]
    fn test_new_manager_from_trait() {
        // Same idea as the previous test, but using the trait constructor.

        let rate_intervals = vec![Segment {
            start: 5.0,
            end: 15.0,
            val: 4.0,
        }];

        let delay_intervals = vec![Segment {
            start: 5.0,
            end: 15.0,
            val: 2.0,
        }];

        let manager =
            <PSegmentationManager as BaseSegmentationManager>::new(
                rate_intervals,
                delay_intervals,
            );

        assert!(manager.booking.is_empty());

        assert_eq!(manager.rate_intervals.len(), 1);
        assert_eq!(manager.rate_intervals[0].start, 5.0);
        assert_eq!(manager.rate_intervals[0].end, 15.0);
        assert_eq!(manager.rate_intervals[0].val, 4.0);

        assert_eq!(manager.delay_intervals.len(), 1);
        assert_eq!(manager.delay_intervals[0].start, 5.0);
        assert_eq!(manager.delay_intervals[0].end, 15.0);
        assert_eq!(manager.delay_intervals[0].val, 2.0);
    }

    #[test]
    fn test_try_init_creates_booking_interval() {
        // After try_init, booking should contain one interval
        // covering the whole contact with default priority -1.

        let rate_intervals = vec![Segment {
            start: 0.0,
            end: 10.0,
            val: 2.0,
        }];

        let delay_intervals = vec![Segment {
            start: 0.0,
            end: 10.0,
            val: 1.0,
        }];

        let mut manager = PSegmentationManager::new(rate_intervals, delay_intervals);

        let contact = ContactInfo::new(1, 2, 0.0, 10.0);

        assert!(manager.try_init(&contact));

        assert_eq!(manager.booking.len(), 1);
        assert_eq!(manager.booking[0].start, 0.0);
        assert_eq!(manager.booking[0].end, 10.0);
        assert_eq!(manager.booking[0].val, -1);
    }

    #[test]
    fn test_dry_run_returns_none_when_not_initialized() {
        // The manager starts with no booking interval.
        // So dry_run_tx should return None.

        let rate_intervals = vec![Segment {
            start: 0.0,
            end: 10.0,
            val: 2.0,
        }];

        let delay_intervals = vec![Segment {
            start: 0.0,
            end: 10.0,
            val: 1.0,
        }];

        let manager = PSegmentationManager::new(rate_intervals, delay_intervals);

        let contact = ContactInfo::new(1, 2, 0.0, 10.0);

        let bundle = Bundle {
            source: 1,
            destinations: vec![2],
            priority: 0,
            size: 4.0,
            expiration: 100.0,
        };

        let result = manager.dry_run_tx(&contact, 0.0, &bundle);

        assert!(result.is_none());
    }

    #[test]
    fn test_dry_run_returns_some_after_init() {
        // After try_init, the manager has one booking interval.
        // Here the bundle can be transmitted, so dry_run_tx should return Some.

        let rate_intervals = vec![Segment {
            start: 0.0,
            end: 10.0,
            val: 2.0,
        }];

        let delay_intervals = vec![Segment {
            start: 0.0,
            end: 10.0,
            val: 1.0,
        }];

        let mut manager = PSegmentationManager::new(rate_intervals, delay_intervals);

        let contact = ContactInfo::new(1, 2, 0.0, 10.0);
        assert!(manager.try_init(&contact));

        let bundle = Bundle {
            source: 1,
            destinations: vec![2],
            priority: 0,
            size: 4.0,
            expiration: 100.0,
        };

        let result = manager.dry_run_tx(&contact, 0.0, &bundle);

        assert!(result.is_some());
    }

    #[test]
    fn test_dry_run_uses_at_time_as_start_when_inside_contact() {
        // If at_time is inside the booking interval,
        // the transmission should start at at_time.

        let rate_intervals = vec![Segment {
            start: 0.0,
            end: 10.0,
            val: 2.0,
        }];

        let delay_intervals = vec![Segment {
            start: 0.0,
            end: 10.0,
            val: 1.0,
        }];

        let mut manager = PSegmentationManager::new(rate_intervals, delay_intervals);

        let contact = ContactInfo::new(1, 2, 0.0, 10.0);
        assert!(manager.try_init(&contact));

        let bundle = Bundle {
            source: 1,
            destinations: vec![2],
            priority: 0,
            size: 4.0,
            expiration: 100.0,
        };

        let result = manager.dry_run_tx(&contact, 3.0, &bundle).unwrap();

        // max(seg.start, at_time) = max(0,3) = 3
        assert_eq!(result.tx_start, 3.0);
    }

    #[test]
    fn test_dry_run_returns_none_when_bundle_is_too_large() {
        // The bundle is too large to finish before the contact end.
        // So dry_run_tx should return None.

        let rate_intervals = vec![Segment {
            start: 0.0,
            end: 10.0,
            val: 1.0,
        }];

        let delay_intervals = vec![Segment {
            start: 0.0,
            end: 10.0,
            val: 1.0,
        }];

        let mut manager = PSegmentationManager::new(rate_intervals, delay_intervals);

        let contact = ContactInfo::new(1, 2, 0.0, 10.0);
        assert!(manager.try_init(&contact));

        let bundle = Bundle {
            source: 1,
            destinations: vec![2],
            priority: 0,
            size: 20.0,
            expiration: 100.0,
        };

        let result = manager.dry_run_tx(&contact, 0.0, &bundle);

        assert!(result.is_none());
    }

    #[test]
    fn test_dry_run_returns_correct_tx_values() {
        // This test checks that the values returned by dry_run_tx are correct.

        let rate_intervals = vec![Segment {
            start: 0.0,
            end: 10.0,
            val: 1.0,
        }];

        let delay_intervals = vec![
            Segment {
                start: 0.0,
                end: 4.0,
                val: 1.0,
            },
            Segment {
                start: 4.0,
                end: 10.0,
                val: 3.0,
            },
        ];

        let mut manager = PSegmentationManager::new(rate_intervals, delay_intervals);

        let contact = ContactInfo::new(1, 2, 0.0, 10.0);
        assert!(manager.try_init(&contact));

        let bundle = Bundle {
            source: 1,
            destinations: vec![2],
            priority: 0,
            size: 5.0,
            expiration: 100.0,
        };

        let result = manager.dry_run_tx(&contact, 0.0, &bundle).unwrap();

        // max(seg.start, at_time)
        assert_eq!(result.tx_start, 0.0);
        // bundle.size = 5, rate_intervals[0].val = 1, so tx_end = 0 + 5 = 5
        assert_eq!(result.tx_end, 5.0);
        // delay_intervals[1].val
        assert_eq!(result.delay, 3.0);
        // booking[0].end = 10.0
        assert_eq!(result.expiration, 10.0);
        // tx_end + delay
        assert_eq!(result.arrival, 8.0);
    }

    #[test]
    fn test_schedule_tx_updates_booking() {
        // schedule_tx should reserve the transmission interval
        // and store the bundle priority in booking.

        let rate_intervals = vec![Segment {
            start: 0.0,
            end: 10.0,
            val: 2.0,
        }];

        let delay_intervals = vec![Segment {
            start: 0.0,
            end: 10.0,
            val: 1.0,
        }];

        let mut manager = PSegmentationManager::new(rate_intervals, delay_intervals);

        let contact = ContactInfo::new(1, 2, 0.0, 10.0);
        assert!(manager.try_init(&contact));

        let bundle = Bundle {
            source: 1,
            destinations: vec![2],
            priority: 2,
            size: 4.0,
            expiration: 100.0,
        };

        let result = manager.schedule_tx(&contact, 0.0, &bundle);
        assert!(result.is_some());

        // The booking interval should be split into:
        // [0,2] with priority 2 and [2,10] with priority -1
        assert_eq!(manager.booking.len(), 2);

        assert_eq!(manager.booking[0].start, 0.0);
        assert_eq!(manager.booking[0].end, 2.0);
        assert_eq!(manager.booking[0].val, 2);

        assert_eq!(manager.booking[1].start, 2.0);
        assert_eq!(manager.booking[1].end, 10.0);
        assert_eq!(manager.booking[1].val, -1);
    }

    #[test]
    fn test_dry_run_skips_booked_interval_for_lower_priority() {
        // After a higher priority bundle is scheduled,
        // a lower priority bundle should start after the booked interval.

        let rate_intervals = vec![Segment {
            start: 0.0,
            end: 10.0,
            val: 2.0,
        }];

        let delay_intervals = vec![Segment {
            start: 0.0,
            end: 10.0,
            val: 1.0,
        }];

        let mut manager = PSegmentationManager::new(rate_intervals, delay_intervals);

        let contact = ContactInfo::new(1, 2, 0.0, 10.0);
        assert!(manager.try_init(&contact));

        let high_priority_bundle = Bundle {
            source: 1,
            destinations: vec![2],
            priority: 2,
            size: 4.0,
            expiration: 100.0,
        };

        assert!(manager.schedule_tx(&contact, 0.0, &high_priority_bundle).is_some());

        let low_priority_bundle = Bundle {
            source: 1,
            destinations: vec![2],
            priority: 1,
            size: 4.0,
            expiration: 100.0,
        };

        let result = manager.dry_run_tx(&contact, 0.0, &low_priority_bundle).unwrap();

        // The first interval [0,2] is booked with priority 2,
        // so a bundle with priority 1 must start at 2.
        assert_eq!(result.tx_start, 2.0);
    }

    //booking + priority 
}
